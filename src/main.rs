use actix_files::NamedFile;
use actix_web::{
    get, http::header::ContentDisposition, http::header::DispositionParam,
    http::header::DispositionType, web, App, Error, HttpServer, Responder,
};
use rusqlite::{params, Connection, NO_PARAMS};
use std::path::Path;
use walkdir::WalkDir;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(net_index)
            .service(net_files)
            .service(net_init)
            .service(net_song_random)
            .service(net_song_by_id)
            .service(net_404)
    })
    .bind(":81")?
    .bind("localhost:81")?
    .run()
    .await
}

#[get("/{id}/{name}/")]
async fn net_index(web::Path((id, name)): web::Path<(u32, String)>) -> impl Responder {
    format!("Hello {}! id:{}", name, id * 3u32)
}

#[get("/files")]
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
            values = format!(
                "{}(\"{}\",\"{}\"),",
                values,
                p,
                e.file_name().to_string_lossy()
            );
            //println!("{}", p);
        });
    println!("{}", size);

    let conn = get_db_con();
    create_table(&conn);
    if !values.is_empty() {
        let statement = &format!(
            "INSERT INTO SONGS (path, name) values {};",
            &values[..values.len() - 1]
        );
        //println!("{}", statement);
        conn.execute(statement, NO_PARAMS).expect("Insertion error");
    }

    return result;
}

#[get("/init")]
async fn net_init() -> impl Responder {
    let conn = get_db_con();
    create_table(&conn)
}

#[get("/songs/{id}")]
async fn net_song_by_id(web::Path(id): web::Path<u32>) -> Result<NamedFile, Error> {
    let c = get_db_con();
    let val: String = c
        .query_row("SELECT path FROM songs WHERE id = ?", params![id], |row| {
            row.get(0)
        })
        .expect("net_song_by_id select path error");

    Ok(get_file_by_name(&val))
}

#[get("/songs/random")]
async fn net_song_random() -> Result<NamedFile, Error> {
    let c = get_db_con();
    let val: String = c
        .query_row(
            "SELECT path FROM songs ORDER BY random() limit 1",
            NO_PARAMS,
            |row| row.get(0),
        )
        .expect("net_song_random select path error");

    Ok(get_file_by_name(&val))
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

fn create_table(c: &Connection) -> String {
    c.execute("DROP TABLE IF EXISTS songs;", NO_PARAMS)
        .expect("Drop error");
    c.execute("CREATE TABLE songs (id INTEGER not null primary key autoincrement, path TEXT unique, name TEXT, play INTEGER);",NO_PARAMS)
        .expect("Table Create error");
    "Success.".to_string()
}

fn get_db_con() -> Connection {
    Connection::open("songdb.sqlite").expect("DB Open error")
}
