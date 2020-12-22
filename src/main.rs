use actix_files::NamedFile;
use actix_web::{
    error::{ErrorInternalServerError, ErrorNotFound},
    get,
    http::header::ContentDisposition,
    http::header::DispositionParam,
    http::header::DispositionType,
    web, App, Error, HttpServer,
};
use rand::{thread_rng, Rng};
use rusqlite::{Connection, NO_PARAMS};
use std::collections::BTreeMap;
use std::path::Path;
use walkdir::WalkDir;

const GL_PORT: i16 = 81i16;
const GL_RATING_BASE: i16 = 400i16;

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
async fn net_files() -> Result<String, Error> {
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
                .unwrap_or("")
                .to_lowercase()
                .ends_with(".mp3")
        })
        .for_each(|e| {
            size += match e.metadata() {
                Ok(ok) => ok.len(),
                Err(_) => 0,
            };
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

    create_table()?;
    if !values.is_empty() {
        let statement = &format!(
            "INSERT INTO SONGS (path, name, rating, vote) values {};",
            &values[..values.len() - 1]
        );

        db_execute(statement)?;
    }

    Ok(result)
}

#[get("/random_id")]
async fn net_get_random_id() -> Result<String, Error> {
    //db_random_id()
    get_weighted_random_id()
}

#[get("/songs/{id}")]
async fn net_song_by_id(web::Path(id): web::Path<u32>) -> Result<NamedFile, Error> {
    let i = &format!("SELECT path FROM songs WHERE id = {}", id);
    let val = db_str_read(i)?;

    get_file_by_name(&val)
}

#[get("/songs/random")]
async fn net_song_random() -> Result<NamedFile, Error> {
    let val = db_random_path()?;

    get_file_by_name(&val)
}

#[get("/upvote/{id}")]
async fn net_song_upvote_by_id(web::Path(id): web::Path<u32>) -> Result<String, Error> {
    let upper = db_uint32_read("select count(*) * 10 from songs")?;
    let mut val = db_uint32_read(&format!("SELECT rating FROM songs WHERE id = {}", id))?;
    val *= 2;

    if val > upper {
        val = upper;
    }

    let i = &format!("Update songs set rating = {} where id = {}", val, id);
    db_execute(i)?;
    Ok("Ok".to_string())
}

#[get("/downvote/{id}")]
async fn net_song_downvote_by_id(web::Path(id): web::Path<u32>) -> Result<String, Error> {
    let mut val = db_uint32_read(&format!("SELECT rating FROM songs WHERE id = {}", id))?;
    val /= 2;

    let i = &format!("Update songs set rating = {} where id = {}", val, id);
    db_execute(i)?;
    Ok("Ok".to_string())
}

#[get("/downvote_mini/{id}")]
async fn net_song_downvote_mini_by_id(web::Path(id): web::Path<u32>) -> Result<String, Error> {
    let mut val = db_uint32_read(&format!("SELECT rating FROM songs WHERE id = {}", id))?;
    val = (val as f64 * 0.66) as u32;

    let i = &format!("Update songs set rating = {} where id = {}", val, id);
    db_execute(i)?;
    Ok("Ok".to_string())
}

#[get("/*")]
async fn net_404() -> Result<String, Error> {
    Err(ErrorNotFound("No pages here."))
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

fn db_uint32_read(sql: &str) -> Result<u32, Error> {
    let c = get_db_con()?;
    match c.query_row::<u32, _, _>(sql, NO_PARAMS, |row| row.get(0)) {
        Ok(o) => Ok(o),
        Err(e) => Err(ErrorInternalServerError(e.to_string())),
    }
}

fn db_str_read(sql: &str) -> Result<String, Error> {
    let c = get_db_con()?;
    match c.query_row::<String, _, _>(sql, NO_PARAMS, |row| row.get(0)) {
        Ok(o) => Ok(o),
        Err(e) => Err(ErrorInternalServerError(e.to_string())),
    }
}

fn db_execute(sql: &str) -> Result<(), actix_web::Error> {
    let conn = get_db_con()?;
    match conn.execute(sql, NO_PARAMS) {
        Ok(_) => Ok(()),
        Err(e) => Err(ErrorInternalServerError(e.to_string())),
    }
}

fn db_random_path() -> Result<String, Error> {
    db_str_read("SELECT path FROM songs ORDER BY ABS(RANDOM() * rating) desc limit 1")
}

fn create_table() -> Result<String, Error> {
    db_execute("DROP TABLE IF EXISTS songs;")?;
    db_execute("CREATE TABLE songs (id INTEGER not null primary key autoincrement, path TEXT unique, name TEXT, rating INTEGER, vote INTEGER);")?;
    Ok("Success.".to_string())
}

fn get_db_con() -> Result<Connection, Error> {
    match Connection::open("songdb.sqlite") {
        Ok(o) => Ok(o),
        Err(e) => Err(ErrorInternalServerError(e.to_string())),
    }
}

struct Entry {
    rating: u32,
    id: u16,
}

fn get_weighted_random_id() -> Result<String, Error> {
    let mut map = BTreeMap::new();

    let mut max = 0u32;

    let c = get_db_con()?;

    let mut stmt = match c.prepare("select rating, id from songs") {
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
    let random = rng.gen_range(0, max + 1);

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
