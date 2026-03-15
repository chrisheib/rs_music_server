use actix_web::{get, web::Data, HttpResponse};
use color_eyre::eyre::Context;
use minijinja::context;
use serde::Serialize;

use crate::{db::db_con, update_manager::db_update, AppState, MyRes, Song};

/// Website with joblist
#[get("/web/jobs")]
async fn net_jobs_web(app: Data<AppState>) -> MyRes<HttpResponse> {
    println!("net_jobs_web");
    db_update()?;
    let vec = read_jobs_db()?;
    // vec.iter().for_each(|s| {
    //     println!(
    //         "Song: {} - {} - {} - {} - {}",
    //         s.id, s.songname, s.artist, s.album, s.length
    //     );
    // });
    let rendered = app.render_template("songlist.html", context! {songs => &vec})?;
    Ok(HttpResponse::Ok().body(rendered))
}

#[derive(Debug, Serialize)]
pub struct Job {
    pub id: i32,
    pub url: String,
    pub output_path: String,
    pub status: String,
    pub step: JobStep,
}

#[derive(Debug, Serialize)]
pub enum JobStep {
    Created,
    Downloading,
    Processing,
    Completed,
    Failed,
}

/// read jobs from db
pub fn read_jobs_db() -> MyRes<Vec<Job>> {
    println!("read_jobs_db called");
    // Placeholder for reading jobs from a database
    Ok(vec![])
}

/// create job endpoint
pub fn create_job(url: &str, output_path: &str) -> MyRes<HttpResponse> {
    println!(
        "create_job called with url: {}, output_path: {}",
        url, output_path
    );
    // Placeholder for job creation logic
    // let job = Job {
    //     id: 1, // This would be generated
    //     url: url.to_string(),
    //     output_path: output_path.to_string(),
    //     status: "created".to_string(),
    // };
    Ok(HttpResponse::Ok().finish())
}

async fn job_worker_wrapper() -> MyRes<()> {
    // let thread = tokio::task::spawn(job_worker).await?;
    Ok(())
}

/// job worker
async fn job_worker() -> MyRes<()> {
    println!("job_worker started");
    // Placeholder for job worker logic
    // e.g., loop to fetch and process jobs from a queue
    let jobs = read_jobs_db()?;
    for job in jobs {
        match job.step {
            JobStep::Created => {
                println!("Processing job id: {}", job.id);
                // download_song(&job.url, &job.output_path)?;
                // Update job status in DB
            }
            _ => {
                println!("Job id: {} is in step: {:?}", job.id, job.step);
            }
        }
    }
    Ok(())
}

/// download, gain correct, save
pub fn download_song(url: &str, output_path: &str) -> MyRes<()> {
    println!("Downloading song from URL: {}", url);
    // Placeholder for download logic
    // e.g., use reqwest to download the file and save it to output_path
    Ok(())
}
