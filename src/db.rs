use std::{fs, path::Path};

use crate::MyRes;
use rusqlite::{Connection, Params};
use stable_eyre::eyre::Context;

pub fn db_select<T, P, F>(sql: &str, params: P, f: F) -> MyRes<T>
where
    P: Params,
    F: FnOnce(&rusqlite::Row<'_>) -> std::result::Result<T, rusqlite::Error>,
{
    let c = db_con()?;
    let mut stmt = c.prepare(sql)?;
    Ok(stmt.query_row(params, f)?)
}

pub fn db_uint32_read(sql: &str) -> MyRes<u32> {
    let c = db_con()?;
    Ok(c.query_row::<u32, _, _>(sql, [], |row| row.get(0))?)
}

pub fn adb_uint32_read(sql: &str) -> MyRes<u32> {
    db_uint32_read(sql)
}

pub fn db_str_read(sql: &str) -> MyRes<String> {
    let c = db_con()?;
    Ok(c.query_row::<String, _, _>(sql, [], |row| row.get(0))?)
}

pub fn db_execute(sql: &str) -> MyRes<()> {
    let conn = db_con()?;
    conn.execute(sql, [])
        .wrap_err(format!("db_execute: {sql}"))?;
    Ok(())
}

pub fn db_con() -> MyRes<Connection> {
    if !Path::new("/music-srv/db/").exists() {
        fs::create_dir_all("/music-srv/db/")?;
    }
    Ok(Connection::open("/music-srv/db/songdb.sqlite")?)
}
