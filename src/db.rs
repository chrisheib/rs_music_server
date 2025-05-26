use std::{fs, path::Path};

use crate::{MyRes, GL_DBDIR};
use color_eyre::eyre::{bail, eyre};
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

pub fn db_execute<P>(sql: &str, params: P) -> MyRes<()>
where
    P: Params,
{
    let conn = db_con()?;
    if let Err(e) = conn.execute(sql, params) {
        println!("db_execute: {sql}, error: {e}");
        return Err(Box::new(e));
    }
    Ok(())
}

pub fn db_con() -> MyRes<Connection> {
    Ok(Connection::open(GL_DBDIR.join("songdb.sqlite"))?)
}
