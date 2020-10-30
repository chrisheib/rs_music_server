use actix_web::{get, web, App, HttpServer, Responder};
use walkdir::WalkDir;

#[get("/{id}/{name}/")]
async fn index(web::Path((id, name)): web::Path<(u32, String)>) -> impl Responder {
    format!("Hello {}! id:{}", name, id * 3u32)
}

#[get("/files/")]
async fn files() -> impl Responder {
    let mut result: String = "".to_string();

    let files = WalkDir::new("E:\\Musik\\");
    files
        .into_iter()
        .filter_map(Result::ok)
        .filter(|f| f.path().to_str().unwrap().to_lowercase().ends_with(".mp3"))
        .for_each(|e| {
            result = format!("{}{}\n", result, e.clone().path().display().to_string());
            // DB-Logik hier!
            //println!("{}", e.path().display().to_string());
        });

    return result;
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(index).service(files))
        .bind("127.0.0.1:81")?
        .run()
        .await
}
