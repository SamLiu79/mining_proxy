use std::{env, path::PathBuf};

use crate::web::{actions, run};
use actix_files::NamedFile;
use actix_web::{self, get, HttpRequest, HttpResponse, Responder, Result, web};
use log::info;

use super::DbPool;

pub async fn index(req: HttpRequest) -> Result<NamedFile> {
    Ok(NamedFile::open("./static/index.html")?)
}

pub async fn greet(req: HttpRequest, pool: web::Data<DbPool>) -> impl Responder {
    let name = req.match_info().get("name").unwrap_or("World");
    // let a = db.state();
    // info!("{:?}",a);
    let pool1 = pool.clone();
    let user = web::block(move || {
        let conn = pool.get()?;
        actions::find_user_by_uid(&conn)
    })
    .await
    .map_err(|e| {
        eprintln!("{}", e);
        HttpResponse::InternalServerError().finish()
    })
    .unwrap()
    .unwrap();
    let db = pool1.get().unwrap();
    run(user, db);

    format!("Hello {}!", &name)
}

#[get("/api/admin_info")]
pub async fn admin_info(req: HttpRequest) -> impl Responder {
//    "{\"code\":200,\"result\":{\"userId\":\"1\",\"username\":\"admin\",\"realName\":\"Admin\",\"avatar\":\"http://dummyimage.com/234x60","desc":"manager","password":"UWDNEDQQB","token":"FLYINHMUYMLKEIPPGSCRNBMLIHDTZCEB","permissions":[{"label":"主控台","value":"dashboard_console"},{"label":"监控页","value":"dashboard_monitor"},{"label":"工作台","value":"dashboard_workplace"},{"label":"基础列表","value":"basic_list"},{"label":"基础列表删除","value":"basic_list_delete"}]},"message":"ok","type":"success"}"
    "{\"code\":200,\"result\":{\"userId\":\"1\",\"username\":\"admin\",\"realName\":\"Admin\",\"avatar\":\"http://dummyimage.com/234x60\",\"desc\":\"manager\",\"password\":\"UWDNEDQQB\",\"token\":\"FLYINHMUYMLKEIPPGSCRNBMLIHDTZCEB\",\"permissions\":[{\"label\":\"主控台\",\"value\":\"dashboard_console\"},{\"label\":\"监控页\",\"value\":\"dashboard_monitor\"},{\"label\":\"工作台\",\"value\":\"dashboard_workplace\"},{\"label\":\"基础列表\",\"value\":\"basic_list\"},{\"label\":\"基础列表删除\",\"value\":\"basic_list_delete\"}]},\"message\":\"ok\",\"type\":\"success\"}"
}


#[get("/api/pool/list")]
pub async fn pool_list(req: HttpRequest,pool: web::Data<DbPool>) -> Result<impl Responder>{
    let pool1 = pool.clone();


    let p = web::block(move || {
        let conn = pool.get()?;
        actions::get_all_pools(&conn)
    })
    .await
    .map_err(|e| {
        eprintln!("{}", e);
        HttpResponse::InternalServerError().finish()
    })
    .unwrap()
    .unwrap();

    
    Ok(web::Json(p))
}
