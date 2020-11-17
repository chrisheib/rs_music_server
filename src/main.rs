use actix_files::NamedFile;
use actix_web::{
    get, http::header::ContentDisposition, http::header::DispositionParam,
    http::header::DispositionType, web, App, Error, HttpServer, Responder,
};
use rusqlite::{Connection, NO_PARAMS};
use std::path::Path;
use walkdir::WalkDir;

const GL_PORT: i16 = 81i16;
const GL_RATING_UPPER: i16 = 500i16;
const GL_RATING_BASE: i16 = 400i16;
const GL_RATING_LOWER: i16 = 300i16;
const GL_UPVOTE_VALUE: i16 = 10i16;
const GL_DOWNVOTE_VALUE: i16 = 10i16;
const GL_DOWNVOTE_MINI_VALUE: i16 = 5i16;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(net_files)
            .service(net_get_random_id)
            .service(net_song_random)
            .service(net_song_by_id)
            .service(net_song_upvote_by_id)
            .service(net_song_downvote_by_id)
            .service(net_song_downvote_mini_by_id)
            .service(net_404)
    })
    .bind(format!(":{}", GL_PORT))?
    .bind(format!("localhost:{}", GL_PORT))?
    .run()
    .await
}

#[get("/reset")]
async fn net_files() -> impl Responder {
    let mut result: String = "".to_string();
    let mut values: String = "".to_string();
    let mut size: u64 = 0;

    let files = WalkDir::new("E:\\Musik\\");
    files
        .into_iter()
        .filter_map(Result::ok)
        .filter(|f| {
            f.path()
                .to_str()
                .expect("File error")
                .to_lowercase()
                .ends_with(".mp3")
        })
        .for_each(|e| {
            size += e.metadata().unwrap().len();
            let p = e.path().display().to_string();
            result = format!("{}{}\n", result, p);
            // Statement-Values aufbauen
            values = format!(
                "{}(\"{}\",\"{}\", {}, 0),",
                values,
                p,
                e.file_name().to_string_lossy(),
                GL_RATING_BASE
            );
            //println!("{}", p);
        });
    println!("{}", size);

    create_table();
    if !values.is_empty() {
        let statement = &format!(
            "INSERT INTO SONGS (path, name, rating, vote) values {};",
            &values[..values.len() - 1]
        );

        db_execute(statement)
    }

    return result;
}

#[get("/random_id")]
async fn net_get_random_id() -> impl Responder {
    db_random_id()
}

#[get("/songs/{id}")]
async fn net_song_by_id(web::Path(id): web::Path<u32>) -> Result<NamedFile, Error> {
    let i = &format!("SELECT path FROM songs WHERE id = {}", id);
    let val = db_str_read(i);

    Ok(get_file_by_name(&val))
}

#[get("/songs/random")]
async fn net_song_random() -> Result<NamedFile, Error> {
    let val = db_random_path();

    Ok(get_file_by_name(&val))
}

#[get("/upvote/{id}")]
async fn net_song_upvote_by_id(web::Path(id): web::Path<u32>) -> impl Responder {
    let val = db_int_read(&format!("SELECT rating FROM songs WHERE id = {}", id));
    if val <= GL_RATING_UPPER - GL_UPVOTE_VALUE {
        let i = &format!(
            "Update songs set rating = rating + {} where id = {}",
            GL_UPVOTE_VALUE, id
        );
        db_execute(i);
    } else {
        let i = &format!(
            "Update songs set rating = {} where id = {}",
            GL_RATING_UPPER, id
        );
        db_execute(i);
    }
    "Ok"
}

#[get("/downvote/{id}")]
async fn net_song_downvote_by_id(web::Path(id): web::Path<u32>) -> impl Responder {
    let val = db_int_read(&format!("SELECT rating FROM songs WHERE id = {}", id));
    if val >= GL_RATING_LOWER + GL_DOWNVOTE_VALUE {
        let i = &format!(
            "Update songs set rating = rating - {} where id = {}",
            GL_DOWNVOTE_VALUE, id
        );
        db_execute(i);
    } else {
        let i = &format!(
            "Update songs set rating = {} where id = {}",
            GL_RATING_LOWER, id
        );
        db_execute(i);
    }
    "Ok"
}

#[get("/downvote_mini/{id}")]
async fn net_song_downvote_mini_by_id(web::Path(id): web::Path<u32>) -> impl Responder {
    let val = db_int_read(&format!("SELECT rating FROM songs WHERE id = {}", id));
    if val >= GL_RATING_LOWER + GL_DOWNVOTE_MINI_VALUE {
        let i = &format!(
            "Update songs set rating = rating - {} where id = {}",
            GL_DOWNVOTE_MINI_VALUE, id
        );
        db_execute(i);
    } else {
        let i = &format!(
            "Update songs set rating = {} where id = {}",
            GL_RATING_LOWER, id
        );
        db_execute(i);
    }
    "Ok"
}

#[get("/*")]
async fn net_404() -> impl Responder {
    "Seite nicht gefunden!"
}

fn get_file_by_name(path: &str) -> NamedFile {
    let p = Path::new(path);
    let file = NamedFile::open(p).expect("file by name error");
    file.use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Inline,
            parameters: vec![DispositionParam::Filename(
                p.file_name().unwrap().to_str().unwrap().to_string(),
            )],
        })
}

fn db_int_read(sql: &str) -> i16 {
    let c = get_db_con();
    let val: i16 = c
        .query_row(sql, NO_PARAMS, |row| row.get(0))
        .expect("db_str_read error");
    val
}

fn db_str_read(sql: &str) -> String {
    let c = get_db_con();
    let val: String = c
        .query_row(sql, NO_PARAMS, |row| row.get(0))
        .expect("db_str_read error");
    val
}

fn db_execute(sql: &str) {
    let conn = get_db_con();
    conn.execute(sql, NO_PARAMS).expect("db_execute error");
}

fn db_random_path() -> String {
    db_str_read("SELECT path FROM songs ORDER BY ABS(RANDOM() * rating) desc limit 1")
}

fn db_random_id() -> String {
    let v = db_int_read("SELECT id FROM songs ORDER BY ABS(RANDOM() * rating) desc limit 1");
    return v.to_string();
}

fn create_table() -> String {
    let c = get_db_con();
    c.execute("DROP TABLE IF EXISTS songs;", NO_PARAMS)
        .expect("Drop error");
    c.execute("CREATE TABLE songs (id INTEGER not null primary key autoincrement, path TEXT unique, name TEXT, rating INTEGER, vote INTEGER);",NO_PARAMS)
        .expect("Table Create error");
    "Success.".to_string()
}

fn get_db_con() -> Connection {
    Connection::open("songdb.sqlite").expect("DB Open error")
}
