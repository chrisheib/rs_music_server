use actix_files::NamedFile;
use actix_web::{
    error::{ErrorInternalServerError, ErrorNotFound},
    get,
    http::header::ContentDisposition,
    http::header::DispositionParam,
    http::header::DispositionType,
    web, App, Error, HttpServer,
};
use db::*;
use json::{object, JsonValue};
use rand::{thread_rng, Rng};
use rusqlite::NO_PARAMS;
use std::collections::BTreeMap;
use std::path::Path;
use update_manager::adb_update;
use walkdir::WalkDir;

mod db;
mod update_manager;

//select "updgedootet", count(*), sum(rating), sum(rating)*1.0 / (select sum(rating) from songs) from  songs where rating > 400
//union
//select "neutral", count(*), sum(rating), sum(rating)*1.0 / (select sum(rating) from songs) from  songs where rating = 400
//union
//select "downgedootet", count(*), sum(rating), sum(rating)*1.0 / (select sum(rating) from songs) from  songs where rating < 400

const GL_PORT: i16 = 81i16;
const GL_RATING_BASE: u16 = 400u16;
const GL_DEBUG_SIZE: bool = false;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(net_update_files)
            .service(net_get_random_id)
            .service(net_song_random)
            .service(net_song_by_id)
            .service(net_song_upvote_by_id)
            .service(net_song_downvote_by_id)
            .service(net_song_downvote_mini_by_id)
            .service(net_songdata_by_id)
            .service(net_songdata_pretty_by_id)
            .service(net_404)
    })
    .bind(format!(":{}", GL_PORT))?
    .bind(format!("localhost:{}", GL_PORT))?
    .run()
    .await
}

#[get("/update")]
async fn net_update_files() -> Result<String, Error> {
    adb_update()?;

    let mut result: String = "".to_string();
    let mut size: u64 = 0;
    let mut values = String::with_capacity(10000 * 300);

    let files = WalkDir::new("E:\\Musik\\");
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

            let songname;
            let artist;
            let album;

            if let Ok(tags) = audiotags::Tag::new()
                .with_tag_type(audiotags::TagType::Id3v2)
                .read_from_path(&path)
            {
                songname = tags.title().unwrap_or_default().to_owned();
                artist = tags.artist().unwrap_or_default().to_owned();
                album = tags.album_title().unwrap_or_default().to_owned();
            } else {
                songname = "".to_string();
                artist = "".to_string();
                album = "".to_string();
            };

            let seconds = get_songlength_secs(&path);
            let length = format_songlength(seconds);
            let rating = GL_RATING_BASE;
            let vote = 0;
            let deleted = 0;

            result = format!("{}{}\n", result, path);
            // Statement-Values aufbauen
            values = format!(
                "{}(\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\", {}, {}, {}, {}),",
                values,
                path.replace("\"", ""),
                filename.replace("\"", ""),
                songname.replace("\"", ""),
                artist.replace("\"", ""),
                album.replace("\"", ""),
                length.replace("\"", ""),
                seconds,
                rating,
                vote,
                deleted
            );
        });
    if GL_DEBUG_SIZE {
        println!("{}", size);
    }

    if !values.is_empty() {
        adb_execute("UPDATE songs set deleted = 1")?;

        let statement = &format!(
            "INSERT INTO songs (path, filename, songname, artist, album, length, seconds, rating, vote, deleted)
            VALUES {}
            ON CONFLICT (path) DO UPDATE SET
            songname=excluded.songname,
            artist=excluded.artist,
            album=excluded.album,
            length=excluded.length,
            seconds=excluded.seconds,
            deleted=excluded.deleted",
            &values[..values.len() - 1]
        );

        std::fs::remove_file("log.txt").unwrap_or_default();
        std::fs::write("log.txt", statement).unwrap_or_default();

        adb_execute(statement)?;
    }

    Ok(result)
}

#[get("/random_id")]
async fn net_get_random_id() -> Result<String, Error> {
    get_weighted_random_id()
}

#[get("/songs/{id}")]
async fn net_song_by_id(web::Path(id): web::Path<u32>) -> Result<NamedFile, Error> {
    let val = get_songpath_by_id(id)?;
    get_file_by_name(&val)
}

#[get("/songs/random")]
async fn net_song_random() -> Result<NamedFile, Error> {
    let id = get_weighted_random_id()?;
    let path = get_songpath_by_id(id.parse::<u32>().unwrap_or_default())?;
    get_file_by_name(&path)
}

#[get("/songdata/{id}")]
async fn net_songdata_by_id(web::Path(id): web::Path<u32>) -> Result<String, Error> {
    let song = get_songdata_json(id)?;
    Ok(song.dump())
}

#[get("/songdata_pretty/{id}")]
async fn net_songdata_pretty_by_id(web::Path(id): web::Path<u32>) -> Result<String, Error> {
    let song = get_songdata_json(id)?;
    Ok(format!("{:#}", song))
}

fn get_songdata_json(id: u32) -> Result<JsonValue, Error> {
    adb_select(
        "select * from songs where id = ?",
        &[id],
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
            })
        },
    )
}

#[get("/upvote/{id}")]
async fn net_song_upvote_by_id(web::Path(id): web::Path<u32>) -> Result<String, Error> {
    let upper = adb_uint32_read("select count(*) * 10 from songs where deleted = 0")?;
    let mut val = adb_uint32_read(&format!("SELECT rating FROM songs WHERE id = {}", id))?;
    val *= 2;

    if val > upper {
        val = upper;
    }

    let i = &format!("Update songs set rating = {} where id = {}", val, id);
    adb_execute(i)?;
    Ok("Ok".to_string())
}

#[get("/downvote/{id}")]
async fn net_song_downvote_by_id(web::Path(id): web::Path<u32>) -> Result<String, Error> {
    let mut val = adb_uint32_read(&format!("SELECT rating FROM songs WHERE id = {}", id))?;
    val /= 2;

    let i = &format!("Update songs set rating = {} where id = {}", val, id);
    adb_execute(i)?;
    Ok("Ok".to_string())
}

#[get("/downvote_mini/{id}")]
async fn net_song_downvote_mini_by_id(web::Path(id): web::Path<u32>) -> Result<String, Error> {
    let mut val = adb_uint32_read(&format!("SELECT rating FROM songs WHERE id = {}", id))?;
    val = (val as f64 * 0.66) as u32;

    let i = &format!("Update songs set rating = {} where id = {}", val, id);
    adb_execute(i)?;
    Ok("Ok".to_string())
}

#[get("/*")]
async fn net_404() -> Result<String, Error> {
    Err(ErrorNotFound("No pages here."))
}

fn get_songpath_by_id(id: u32) -> Result<String, Error> {
    let i = &format!("SELECT path FROM songs WHERE id = {}", id);
    adb_str_read(i)
}

fn get_songlength_secs(path: &str) -> u64 {
    let path = Path::new(path);
    let duration = mp3_duration::from_path(&path).unwrap_or_default();
    duration.as_secs()
}

fn format_songlength(seconds: u64) -> String {
    let mins = seconds / 60;
    let secs = seconds % 60;
    if mins >= 60 {
        let hours = mins / 60;
        let mins = mins / 60;
        format!("{}:{:0>2}:{:0>2}", hours, mins, secs)
    } else {
        format!("{:0>1}:{:0>2}", mins, secs)
    }
}

fn get_file_by_name(path: &str) -> Result<NamedFile, Error> {
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

struct Entry {
    rating: u32,
    id: u16,
}

fn get_weighted_random_id() -> Result<String, Error> {
    let mut map = BTreeMap::new();

    let mut max = 0u32;

    let c = adb_con()?;

    let mut stmt = match c.prepare("select rating, id from songs where deleted = 0") {
        Ok(o) => Ok(o),
        Err(e) => Err(ErrorInternalServerError(e.to_string())),
    }?;

    let row_res = stmt.query_map(NO_PARAMS, |row| {
        Ok(Entry {
            rating: row.get(0).expect("get rating error"),
            id: row.get(1).expect("get id error"),
        })
    });

    let rows = match row_res {
        Ok(o) => Ok(o),
        Err(e) => Err(ErrorInternalServerError(e.to_string())),
    }?;

    // get data from db
    for row in rows {
        let a = match row {
            Ok(o) => Ok(o),
            Err(e) => Err(ErrorInternalServerError(e.to_string())),
        }?;

        // put data into map (count), add values
        map.insert(max, a.id);

        max += a.rating;
    }

    // generate random number 0 .. max
    let mut rng = thread_rng();
    let random = rng.gen_range(0..max + 1);

    let res = map.range(..random).next_back();

    let final_res;

    match res {
        None => {
            let zeroth = map.iter().next();
            match zeroth {
                Some(s) => final_res = s,
                None => return Err(ErrorInternalServerError("Zero-th entry could't be opened!")),
            };
        }
        Some(s) => final_res = s,
    }

    let c = *final_res.1;
    Ok(c.to_string())

    // https://stackoverflow.com/a/49600137/12591389 :
    //
    // println!("maximum in map less than {}: {:?}",
    // key, map.range(..key).next_back().unwrap());
}

pub fn errconv<T>(r: stable_eyre::Result<T>) -> Result<T, Error> {
    match r {
        Ok(o) => Ok(o),
        Err(e) => Err(ErrorInternalServerError(e)),
    }
}
