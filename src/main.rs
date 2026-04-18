use actix_files::NamedFile;
use actix_multipart::Multipart;
use actix_web::{
    error::ErrorNotFound,
    get,
    http::header::{ContentDisposition, DispositionParam, DispositionType},
    post,
    web::{self, Data, Json},
    App, HttpResponse, HttpServer,
};
use color_eyre::eyre::{eyre, Context};
use color_eyre::{install, Result};
use db::*;
use futures_util::StreamExt;
use itertools::Itertools;
use json::{object, JsonValue};
use lazy_static::lazy_static;
use minijinja::{context, Environment};
use minijinja::{path_loader, Value};
use minijinja_autoreload::AutoReloader;
use rand::{distributions::WeightedIndex, prelude::Distribution, thread_rng};
use rusqlite::Statement;
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, File},
    io::Write,
    path::PathBuf,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::join;
use walkdir::WalkDir;

use crate::update_manager::db_update;

mod db;
mod dl;
mod update_manager;

type MyRes<T> = Result<T, Box<dyn std::error::Error>>;

type Mylist = Arc<Mutex<Vec<i32>>>;

lazy_static! {
        static ref LAST_SONGS: Mylist = Arc::new(Mutex::new(Vec::new()));
    static ref GL_PORT: i16 = env::var("PORT")
        .map(|v| v.parse::<i16>().unwrap_or(3000))
        .unwrap_or(3000);
    static ref GL_INTERNAL_PORT: i16 = env::var("PORT_INTERNAL")
        .map(|v| v.parse::<i16>().unwrap_or(3001))
        .unwrap_or(3001);
    static ref GL_MUSICDIR: PathBuf = env::var("MUSICDIR").map(PathBuf::from).unwrap_or(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("music")
    );
    // /music-srv/db/
    static ref GL_DBDIR: PathBuf = env::var("DBDIR").map(PathBuf::from).unwrap_or(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    );
    static ref GL_UPLOADDIR: PathBuf = env::var("UPLOADDIR").map(PathBuf::from).unwrap_or(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("music").join("upload")
    );
}

const GL_INSERT_SONG_STMT: &str = "INSERT INTO songs (path, filename, songname, artist, album, length, seconds, rating, vote, deleted)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT (path) DO UPDATE SET
            songname=excluded.songname,
            artist=excluded.artist,
            album=excluded.album,
            length=excluded.length,
            seconds=excluded.seconds,
            deleted=excluded.deleted";

const GL_RATING_BASE: i32 = 2i32;
const GL_DEFAULT_RATING_SCALE: f32 = 2.5f32;
const GL_DEBUG_SIZE: bool = false;
const GL_REPLAY_PROTECTION: usize = 15;
const BUILD_TIMESTAMP_FILENAME: &str = "build-timestamp.txt";

struct AppState {
    template_env: AutoReloader,
    build_timestamp: String,
}

impl AppState {
    fn render_template(&self, name: &str, ctx: Value) -> Result<String> {
        let env = self.template_env.acquire_env()?;
        let template = env.get_template(name)?;
        let rendered = template.render(ctx)?;
        Ok(rendered)
    }

    /// Returns the build timestamp shown in templates so deployments can be verified in the UI.
    fn build_timestamp(&self) -> &str {
        &self.build_timestamp
    }
}

/// Loads the image build timestamp so the web UI can expose which deployment is serving requests.
fn load_build_timestamp() -> String {
    let build_timestamp_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(BUILD_TIMESTAMP_FILENAME);

    match fs::read_to_string(build_timestamp_path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return "dev".to_string();
            }
            trimmed.to_string()
        }
        Err(_) => "dev".to_string(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    install()?;
    println!("http://localhost:{}", *GL_PORT);
    println!("http://localhost:{}", *GL_INTERNAL_PORT);
    println!("MUSICDIR: {}", GL_MUSICDIR.to_str().unwrap_or_default());
    let _job_worker_handle = dl::start_job_worker();
    let build_timestamp = load_build_timestamp();

    let ext = web::Data::new(AppState {
        template_env: AutoReloader::new(|notifier| {
            let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
            let mut env = Environment::new();
            env.set_loader(path_loader(&template_path));

            notifier.watch_path(&template_path, true);
            Ok(env)
        }),
        build_timestamp,
    });

    let ext1 = ext.clone();

    let exposed_server_handle = HttpServer::new(move || {
        App::new()
            .configure(dl::configure_routes)
            .service(net_songlist)
            .service(net_get_random_id)
            .service(net_get_random_id_with_scale)
            .service(net_song_random)
            .service(net_song_by_id)
            .service(net_song_upvote_by_id)
            .service(net_song_downvote_by_id)
            .service(net_songdata_by_id)
            .service(net_songdata_pretty_by_id)
            .service(net_404)
            .service(net_ping)
            .app_data(ext1.clone())
    })
    .bind(format!("0.0.0.0:{}", *GL_PORT))?
    .run();

    let exposed_server_handle_2 = HttpServer::new(move || {
        App::new()
            .configure(dl::configure_routes)
            .service(net_update_files)
            .service(net_songlist)
            .service(net_get_random_id)
            .service(net_get_random_id_with_scale)
            .service(net_song_random)
            .service(net_song_by_id)
            .service(net_song_upvote_by_id)
            .service(net_song_downvote_by_id)
            .service(net_songdata_by_id)
            .service(net_songdata_pretty_by_id)
            .service(net_404)
            .service(net_ping)
            // .service(net_index)
            .service(net_songlist_web)
            .service(net_upload)
            .service(net_update_songdata_by_id_post)
            .app_data(ext.clone())
    })
    .bind(format!("0.0.0.0:{}", *GL_INTERNAL_PORT))?
    .run();
    println!(
        "Starting servers on ports {} and {}",
        *GL_PORT, *GL_INTERNAL_PORT
    );
    let (res1, res2) = join!(exposed_server_handle, exposed_server_handle_2);
    res1?;
    res2?;
    Ok(())
}

// #[get("/")]
// async fn net_index(app: Data<AppState>) -> MyRes<HttpResponse> {
//     println!("net_index");
//     // let ctx = context! (
//     // title => "Hello World",
//     // name =>  "World",
//     // );
//     // let rendered = app.render_template("index.html", ctx)?;
//     let rendered = app.render_template("songlist.html", context! {})?;
//     Ok(HttpResponse::Ok().body(rendered))
// }

#[get("/ping")]
async fn net_ping() -> MyRes<String> {
    println!("net_ping");
    Ok("pong".to_string())
}

#[get("/update")]
async fn net_update_files() -> MyRes<String> {
    println!("net_update_files");
    db_update()?;

    let mut size: u64 = 0;
    let mut count = 0;

    let mut db = db_con()?;
    let b = db.transaction().wrap_err("transaction")?;
    let mut s = b.prepare(GL_INSERT_SONG_STMT).wrap_err("prepare")?;

    b.execute("UPDATE songs set deleted = 1", [])
        .wrap_err("delete")?;

    let files = WalkDir::new(&*GL_MUSICDIR);
    files
        .into_iter()
        .filter_map(Result::ok)
        .filter(|f| {
            f.path()
                .to_str()
                .unwrap_or("")
                .to_lowercase()
                .ends_with(".mp3")
        })
        .for_each(|entry| {
            if GL_DEBUG_SIZE {
                size += match entry.metadata() {
                    Ok(ok) => ok.len(),
                    Err(_) => 0,
                };
            }
            let path = entry.path().display().to_string();
            let filename = entry.file_name().to_string_lossy();

            add_song_in_transaction(&path, &filename, &mut s);

            count += 1;
            if count % 1000 == 0 {
                println!("net_update_files count: {count}");
            }
        });
    if GL_DEBUG_SIZE {
        println!("{size}");
    }

    drop(s);
    b.commit().wrap_err("commit")?;

    Ok("Ok".to_string())
}

fn add_song_in_transaction(path: &str, filename: &str, s: &mut Statement) {
    println!("add_song_in_transaction({path}, {filename})");
    let songname;
    let artist;
    let album;

    if let Ok(tags) = audiotags::Tag::new()
        .with_tag_type(audiotags::TagType::Id3v2)
        .read_from_path(path)
    {
        songname = tags.title().unwrap_or_default().to_owned();
        artist = tags.artist().unwrap_or_default().to_owned();
        album = tags.album_title().unwrap_or_default().to_owned();
    } else {
        songname = "".to_string();
        artist = "".to_string();
        album = "".to_string();
    };

    let seconds = get_songlength_secs(path);
    let length = format_songlength(seconds);
    let rating = GL_RATING_BASE;
    let vote = 0;
    let deleted = 0;

    // result = format!("{result}{path}\n");
    // Statement-Values aufbauen
    let values = (
        path, filename, songname, artist, album, length, seconds, rating, vote, deleted,
    );

    s.execute(values).unwrap();
}

#[get("/random_id/{scale}")]
async fn net_get_random_id_with_scale(scale: web::Path<f32>) -> MyRes<String> {
    println!("net_get_random_id_with_scale({scale})");
    db_update()?;
    get_weighted_random_id(*scale)
}

#[get("/random_id")]
async fn net_get_random_id() -> MyRes<String> {
    println!("net_get_random_id");
    db_update()?;
    get_weighted_random_id(GL_DEFAULT_RATING_SCALE)
}

#[derive(Serialize)]
struct Song {
    id: i32,
    path: String,
    filename: String,
    songname: String,
    artist: String,
    album: String,
    length: String,
    seconds: i32,
    rating: i32,
    vote: i32,
    times_played: i32,
}

#[get("/songs")]
async fn net_songlist() -> MyRes<web::Json<Vec<Song>>> {
    println!("net_songlist");
    db_update()?;
    let sql = "select id, path, filename, songname, artist, album, length, seconds, rating, vote, times_played from songs where deleted = 0";

    let c = db_con()?;
    let mut stmt = c.prepare(sql).wrap_err("prepare")?;
    let vec = stmt.query_map([], |row| {
        Ok(Song {
            id: row.get::<_, i32>(0)?,
            path: row.get::<_, String>(1)?,
            filename: row.get::<_, String>(2)?,
            songname: row.get::<_, String>(3)?,
            artist: row.get::<_, String>(4)?,
            album: row.get::<_, String>(5)?,
            length: row.get::<_, String>(6)?,
            seconds: row.get::<_, i32>(7)?,
            rating: row.get::<_, i32>(8)?,
            vote: row.get::<_, i32>(9)?,
            times_played: row.get::<_, i32>(10)?,
        })
    });

    let vec = vec?.collect::<Result<Vec<_>, _>>()?;

    Ok(Json(vec))
}

#[get("/")]
async fn net_songlist_web(app: Data<AppState>) -> MyRes<HttpResponse> {
    println!("net_songlist_web");
    db_update()?;
    let sql = "select id, path, filename, songname, artist, album, length, seconds, rating, vote, times_played from songs where deleted = 0";

    let c = db_con()?;
    let mut stmt = c.prepare(sql).wrap_err("prepare")?;
    let vec = stmt
        .query_map([], |row| {
            Ok(Song {
                id: row.get::<_, i32>(0)?,
                path: row.get::<_, String>(1)?,
                filename: row.get::<_, String>(2)?,
                songname: row.get::<_, String>(3)?,
                artist: row.get::<_, String>(4)?,
                album: row.get::<_, String>(5)?,
                length: row.get::<_, String>(6)?,
                seconds: row.get::<_, i32>(7)?,
                rating: row.get::<_, i32>(8)?,
                vote: row.get::<_, i32>(9)?,
                times_played: row.get::<_, i32>(10)?,
            })
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()?;
    // vec.iter().for_each(|s| {
    //     println!(
    //         "Song: {} - {} - {} - {} - {}",
    //         s.id, s.songname, s.artist, s.album, s.length
    //     );
    // });
    let rendered = app.render_template("songlist.html", context! {songs => &vec})?;
    Ok(HttpResponse::Ok().body(rendered))
}

#[get("/songs/{id}")]
async fn net_song_by_id(id: web::Path<u32>) -> MyRes<NamedFile> {
    println!("net_song_by_id({id})");
    db_update()?;
    let id = id.into_inner();
    increase_times_played(id)?;
    let val = get_songpath_by_id(id)?;
    get_file_by_name(&val)
}

#[get("/songs/random")]
async fn net_song_random() -> MyRes<NamedFile> {
    println!("net_song_random");
    db_update()?;
    let id = get_weighted_random_id(GL_DEFAULT_RATING_SCALE)?
        .parse::<u32>()
        .unwrap_or_default();
    let path = get_songpath_by_id(id)?;
    increase_times_played(id)?;
    get_file_by_name(&path)
}

#[get("/songdata/{id}")]
async fn net_songdata_by_id(id: web::Path<u32>) -> MyRes<String> {
    println!("net_songdata_by_id({id})");
    db_update()?;
    let id = id.into_inner();
    let song = get_songdata_json(id)?;
    Ok(song.dump())
}

#[get("/songdata_pretty/{id}")]
async fn net_songdata_pretty_by_id(id: web::Path<u32>) -> MyRes<String> {
    println!("net_songdata_pretty_by_id({id})");
    db_update()?;
    let id = id.into_inner();
    let song = get_songdata_json(id)?;
    Ok(format!("{song:#}"))
}

fn get_songdata_json(id: u32) -> MyRes<JsonValue> {
    println!("get_songdata_json");
    db_select(
        "select * from songs where id = ?",
        [id],
        |row| -> Result<json::JsonValue, rusqlite::Error> {
            Ok(object! {
                id: row.get(0).unwrap_or(0),
                path: row.get(1).unwrap_or_else(|_a| "".to_string()),
                filename: row.get(2).unwrap_or_else(|_a| "".to_string()),
                songname: row.get(3).unwrap_or_else(|_a| "".to_string()),
                artist: row.get(4).unwrap_or_else(|_a| "".to_string()),
                album: row.get(5).unwrap_or_else(|_a| "".to_string()),
                length: row.get(6).unwrap_or_else(|_a| "".to_string()),
                seconds: row.get(7).unwrap_or(0),
                rating: row.get(8).unwrap_or(0),
                vote: row.get(9).unwrap_or(0),
                times_played: row.get(11).unwrap_or(0)
            })
        },
    )
}

#[get("/upvote/{id}")]
async fn net_song_upvote_by_id(id: web::Path<u32>) -> MyRes<String> {
    println!("net_song_upvote_by_id({id})");
    db_update()?;
    let id = id.into_inner();
    let mut val = adb_uint32_read(&format!("SELECT rating FROM songs WHERE id = {id}"))?;
    if val < 7 {
        val += 1;
        let i = "Update songs set rating = ? where id = ?";
        db_execute(i, (val, id))?;
    }
    Ok(format!("Upvoted {id}. New Score: {val}"))
}

#[get("/downvote/{id}")]
async fn net_song_downvote_by_id(id: web::Path<u32>) -> MyRes<String> {
    println!("net_song_downvote_by_id({id})");
    db_update()?;
    let id = id.into_inner();
    let mut val = adb_uint32_read(&format!("SELECT rating FROM songs WHERE id = {id}"))?;
    if val > 0 {
        val -= 1;
        let i = "Update songs set rating = ? where id = ?";
        db_execute(i, (val, id))?;
    }
    Ok(format!("Downvoted {id}. New Score: {val}"))
}

#[get("/*")]
async fn net_404() -> MyRes<String> {
    println!("net_404");
    Err(ErrorNotFound("No pages here.").into())
}

fn get_songpath_by_id(id: u32) -> MyRes<String> {
    println!("get_songpath_by_id({id})");
    let i = &format!("SELECT path FROM songs WHERE id = {id}");
    db_str_read(i)
}

fn get_songlength_secs(path: &str) -> u64 {
    let path = Path::new(path);
    let duration = mp3_duration::from_path(path).unwrap_or_default();
    duration.as_secs()
}

fn increase_times_played(id: u32) -> MyRes<()> {
    println!("increase_times_played({id})");
    let i = "Update songs set times_played = times_played + 1 where id = ?";
    db_execute(i, [id])
}

fn format_songlength(seconds: u64) -> String {
    let mins = seconds / 60;
    let secs = seconds % 60;
    if mins >= 60 {
        let hours = mins / 60;
        let mins = mins / 60;
        format!("{hours}:{mins:0>2}:{secs:0>2}")
    } else {
        format!("{mins:0>1}:{secs:0>2}")
    }
}

fn get_file_by_name(path: &str) -> MyRes<NamedFile> {
    println!("get_file_by_name");
    let p = Path::new(path);

    let file = NamedFile::open(p)?;

    let filename = p
        .file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or_default();

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Inline,
            parameters: vec![DispositionParam::Filename(filename.to_owned())],
        }))
}

fn get_weighted_random_id(scale: f32) -> MyRes<String> {
    println!("get_weighted_random_id");
    let c = db_con()?;

    let mut stmt = c.prepare("select rating, id from songs where deleted = 0 and rating > 0")?;

    let rows = stmt.query_map([], |row| -> Result<(u32, i32), rusqlite::Error> {
        Ok((row.get::<usize, u32>(0)?, row.get::<usize, i32>(1)?))
    })?;

    let map = rows
        .into_iter()
        .collect::<Result<Vec<(u32, i32)>, _>>()?
        .into_iter()
        .map(|a| {
            let rating = scale_rating(scale, a.0 as i32 - 1);
            (rating, a.1)
        })
        .collect_vec();

    let mut c: i32;

    let lasts = LAST_SONGS.clone();
    let Ok(mut inner) = lasts.lock() else {
        Err(eyre!("Could not acquire mutex!"))?;
        unreachable!();
    };
    loop {
        c = rng(&map)?;

        if !inner.contains(&c) {
            if inner.len() >= GL_REPLAY_PROTECTION {
                inner.remove(0);
            }
            inner.push(c);
            break;
        } else {
            println!("{c} ist schon in der Liste!");
        }
    }
    drop(inner);

    Ok(c.to_string())
}

pub fn rng(map: &[(usize, i32)]) -> MyRes<i32> {
    // println!("rng");
    let res = WeightedIndex::new(map.iter().map(|item| item.0))?;
    let index = res.sample(&mut thread_rng());
    let id = map[index].1;

    // println!("rng return: len {}, index: {index}, id: {id}", map.len());
    Ok(id)
}

#[post("/upload")]
async fn net_upload(mut payload: Multipart) -> HttpResponse {
    println!("net_upload");
    if let Some(field) = payload.next().await {
        println!("net_upload field | {:?}", field);
        if let Ok(mut field) = field {
            let filename = field
                .content_disposition()
                .unwrap()
                .get_filename()
                .unwrap_or("default.mp3")
                .to_owned();
            let filepath = GL_UPLOADDIR.join(&filename);
            println!("filename: {filename}, filepath: {filepath:?}");
            let mut file = File::create(&filepath).expect("Failed to create file");

            while let Some(chunk) = field.next().await {
                if let Ok(chunk) = chunk {
                    file.write_all(&chunk).expect("Failed to write to file");
                }
            }
            println!("File saved: {:?}", filepath);

            if let Err(e) = db_update() {
                return HttpResponse::InternalServerError()
                    .body(format!("Failed to update database: {}", e));
            }

            let Ok(mut db) = db_con() else {
                return HttpResponse::InternalServerError().body("Failed to connect to database");
            };

            let t = db.transaction().unwrap();
            let mut s = t.prepare(GL_INSERT_SONG_STMT).unwrap();
            add_song_in_transaction(filepath.to_str().unwrap(), &filename, &mut s);
            drop(s);
            t.commit().unwrap();
            let new_id = db.last_insert_rowid();
            println!("new id: {new_id}");
            return HttpResponse::Ok().body(format!("File uploaded successfully, id: {new_id}"));
        }
    }
    HttpResponse::BadRequest().body("Failed to upload file")
}

#[derive(Serialize, Deserialize, Debug)]
struct UpdateSongData {
    songname: String,
    artist: String,
    album: String,
    rating: u8,
}

/// Persists song metadata edits from the web UI after normalizing user-entered text fields.
#[post("/songdata/{id}")]
async fn net_update_songdata_by_id_post(
    id: web::Path<u32>,
    data: Json<UpdateSongData>,
) -> MyRes<String> {
    println!("net_songdata_by_id_post({id})");
    db_update()?;
    let d = data.into_inner();
    let songname = d.songname.trim().to_owned();
    let artist = d.artist.trim().to_owned();
    let album = d.album.trim().to_owned();
    let id = id.into_inner();

    let sql = "UPDATE songs SET songname = ?, artist = ?, album = ?, rating = ? WHERE id = ?";

    db_execute(sql, (&songname, &artist, &album, &d.rating, &id))?;

    Ok(format!("Updated song with ID: {id}"))
}

fn scale_rating(scale: f32, weight: i32) -> usize {
    // Scale the rating based on the provided scale factor
    scale.powi(weight).ceil() as usize
}

#[test]
/// takes forwever, call with cargo test --release
fn test() {
    use itertools::Itertools;
    use rand::Rng;
    use std::collections::HashMap;
    use std::{thread, time::Duration};
    const SCALE: f32 = 2.5;

    // create test song entries
    let mut map = Vec::new();
    let mut rand = rand::thread_rng();
    for i in 0..5000 {
        let weight = rand.gen_range(0..=6);
        let rating = scale_rating(SCALE, weight);
        map.push((rating, i));
    }

    // run the rng function and store calls in a HashMap
    let mut hash = HashMap::<(usize, i32), usize>::new();

    for r in 0..10_000_000 {
        let out = rng(&map).unwrap_or_default();
        *hash.entry((map[out as usize].0, out)).or_insert(0) += 1;
        if r % 1_000_000 == 0 {
            mylog(&format!("rng call {r} done"));
        }
    }

    let mut v = hash.iter().collect_vec();
    v.sort_unstable_by_key(|a| a.1);
    v = v.into_iter().rev().collect_vec();
    for r in 0..7 {
        let m = v
            .iter()
            .filter(|((a, _), _)| *a == scale_rating(SCALE, r))
            .collect_vec();

        mylog(&format!("r: {:?}", m.last()));
        mylog(&format!("r: {:?}", m.first()));
    }

    thread::sleep(Duration::from_secs(1));
}

#[test]
fn get_scale() {
    mylog("scale");
    let count_1 = db_uint32_read("select count(*) from songs where rating = 1").unwrap_or_default();
    mylog(&format!("count_1: {count_1}"));
    let count_2 = db_uint32_read("select count(*) from songs where rating = 2").unwrap_or_default();
    mylog(&format!("count_2: {count_2}"));
    let count_3 = db_uint32_read("select count(*) from songs where rating = 3").unwrap_or_default();
    mylog(&format!("count_3: {count_3}"));
    let count_4 = db_uint32_read("select count(*) from songs where rating = 4").unwrap_or_default();
    mylog(&format!("count_4: {count_4}"));
    let count_5 = db_uint32_read("select count(*) from songs where rating = 5").unwrap_or_default();
    mylog(&format!("count_5: {count_5}"));
    let count_6 = db_uint32_read("select count(*) from songs where rating = 6").unwrap_or_default();
    mylog(&format!("count_6: {count_6}"));
    let count_7 = db_uint32_read("select count(*) from songs where rating = 7").unwrap_or_default();
    mylog(&format!("count_7: {count_7}"));

    for i in 1..100 {
        let lower = scale_rating(i as f32 / 10.0, 1) as u32 * count_1
            + scale_rating(i as f32 / 10.0, 2) as u32 * count_2
            + scale_rating(i as f32 / 10.0, 3) as u32 * count_3
            + scale_rating(i as f32 / 10.0, 4) as u32 * count_4;
        let upper = scale_rating(i as f32 / 10.0, 5) as u32 * count_5
            + scale_rating(i as f32 / 10.0, 6) as u32 * count_6
            + scale_rating(i as f32 / 10.0, 7) as u32 * count_7;
        mylog(&format!("scale: {i}, lower: {lower}, upper: {upper}"));
        if upper > lower {
            mylog("done!");
            break;
        }
    }
}

#[allow(dead_code)]
fn mylog(s: &str) {
    // open log.txt and append
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("log.txt")
        .unwrap_or_else(|_| {
            println!("Could not open log.txt");
            std::process::exit(1);
        });
    if let Err(e) = writeln!(file, "{}", s) {
        println!("Could not write to log.txt: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::rng;
    #[test]
    fn test_vec_rng() {
        println!("test_vec_rng");
        let map = vec![
            (100, 100i32),
            (200, 200i32),
            (300, 300i32),
            (100, 400i32),
            (50, 500i32),
            (250, 600i32),
            (100, i32::MAX),
            (100, i32::MIN),
            (100, 0),
        ];

        let mut hash = HashMap::<i32, usize>::new();

        for _ in 0..1300000 {
            let out = rng(&map).unwrap_or_default();
            let val = *hash.get(&out).unwrap_or(&0);
            hash.insert(out, val + 1);
        }
        println!("{hash:#?}");
        assert!(*hash.get(&100).unwrap_or(&0) > 95_000);
        assert!(*hash.get(&100).unwrap_or(&0) < 105_000);

        assert!(*hash.get(&200).unwrap_or(&0) > 195_000);
        assert!(*hash.get(&200).unwrap_or(&0) < 205_000);

        assert!(*hash.get(&300).unwrap_or(&0) > 295_000);
        assert!(*hash.get(&300).unwrap_or(&0) < 305_000);

        assert!(*hash.get(&400).unwrap_or(&0) > 95_000);
        assert!(*hash.get(&400).unwrap_or(&0) < 105_000);

        assert!(*hash.get(&i32::MAX).unwrap_or(&0) > 95_000);
        assert!(*hash.get(&i32::MAX).unwrap_or(&0) < 105_000);

        assert!(*hash.get(&i32::MIN).unwrap_or(&0) > 95_000);
        assert!(*hash.get(&i32::MIN).unwrap_or(&0) < 105_000);

        assert!(*hash.get(&0).unwrap_or(&0) > 95_000);
        assert!(*hash.get(&0).unwrap_or(&0) < 105_000);

        assert!(*hash.get(&500).unwrap_or(&0) > 45_000);
        assert!(*hash.get(&500).unwrap_or(&0) < 55_000);

        assert!(*hash.get(&600).unwrap_or(&0) > 245_000);
        assert!(*hash.get(&600).unwrap_or(&0) < 255_000);
    }
}
