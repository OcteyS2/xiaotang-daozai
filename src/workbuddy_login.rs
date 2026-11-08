//! 默认浏览器登录：官网生成一次性请求，岛仔轮询对应的授权结果。
use super::{Auth,Session,Snapshot,Failure,slot,query,save_session};
use serde_json::Value;
use std::io::Read;
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};
const ORIGIN:&str="https://www.codebuddy.cn";

fn read(response:ureq::Response)->Result<Value,String>{
    serde_json::from_reader(response.into_reader().take(2*1024*1024)).map_err(|_|"Invalid website login response".into())
}
fn response(value:Value)->Result<Value,String>{
    if value["code"].as_i64()!=Some(0){return Err("WorkBuddy can't create a login request right now. Please try again later".into());}
    value.get("data").cloned().ok_or_else(||"Website returned no login request".into())
}
fn login_request(value:Value)->Result<(String,String),String>{
    let data=response(value)?;
    let state=data["state"].as_str().filter(|s|!s.is_empty()).ok_or("Website returned no login ID")?;
    let link=data["authUrl"].as_str().ok_or("Website returned no login URL")?;
    let url=url::Url::parse(link).map_err(|_|"Invalid website login URL")?;
    if url.origin().ascii_serialization()!=ORIGIN || url.path()!="/login"
        || !url.query_pairs().any(|(k,v)|k=="state"&&v==state){
        return Err("Website login URL doesn't match the request".into());
    }
    Ok((state.into(),link.into()))
}
#[cfg(windows)]
fn open_browser(url:&str)->Result<(),String>{
    use windows::{core::{w,PCWSTR},Win32::UI::{Shell::ShellExecuteW,WindowsAndMessaging::SW_SHOWNORMAL}};
    let wide=url.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let result=unsafe{ShellExecuteW(None,w!("open"),PCWSTR(wide.as_ptr()),PCWSTR::null(),PCWSTR::null(),SW_SHOWNORMAL)};
    if result.0 as isize<=32{return Err("Can't open the default browser. Please check Windows default app settings".into());}
    Ok(())
}
#[cfg(not(windows))]
fn open_browser(_url:&str)->Result<(),String>{Err("WorkBuddy login is Windows-only for now".into())}

pub(super) fn connect(expected:Option<&str>,cancel:Arc<AtomicBool>)->Result<(String,Snapshot),String>{
    let agent=super::super::http_agent();
    let value=read(agent.post(&format!("{ORIGIN}/v2/plugin/auth/state?platform=daozai"))
        .set("X-No-Authorization","true").set("X-No-User-Id","true").set("Content-Type","application/json")
        .send_string("{}").map_err(|_|"Can't create WorkBuddy login request")?)?;
    let (state,link)=login_request(value)?;
    open_browser(&link)?;
    wait_for_login(&agent,&state,expected,cancel)
}
pub(super) fn wait_for_login(agent:&ureq::Agent,state:&str,expected:Option<&str>,cancel:Arc<AtomicBool>)->Result<(String,Snapshot),String>{
    let mut poll=url::Url::parse(&format!("{ORIGIN}/v2/plugin/auth/token")).unwrap();
    poll.query_pairs_mut().append_pair("state",&state);
    let start=Instant::now();
    while start.elapsed()<Duration::from_secs(600){
        for _ in 0..20{
            if cancel.load(Ordering::Relaxed){return Err("WorkBuddy login canceled".into());}
            std::thread::sleep(Duration::from_millis(100));
        }
        let result=agent.get(poll.as_str()).set("X-No-Authorization","true").call();
        let value:Value=match result{
            Ok(r)=>read(r)?,
            Err(ureq::Error::Status(400,r))=>read(r)?,
            Err(ureq::Error::Transport(_))=>continue,
            Err(_)=>return Err("WorkBuddy login request expired. Please reconnect".into()),
        };
        if value["code"].as_i64()==Some(11217){continue;}
        let auth:Auth=serde_json::from_value(value["data"].clone()).map_err(|_|"Website returned no valid authorization. Please reconnect")?;
        if auth.access_token.is_empty(){return Err("Website login authorization is empty. Please reconnect".into());}
        let mut session=Session{origin:ORIGIN.into(),ua:String::new(),cookie:String::new(),auth:Some(auth)};
        let accounts=super::with_transport_retry(&agent,ORIGIN,|agent|super::api(agent,&session,"/v2/plugin/accounts")).map_err(Failure::message)?;
        let list=accounts.pointer("/data/accounts").and_then(Value::as_array).ok_or("Website returned no account list")?;
        let personal=list.iter().filter(|a|a["enterpriseId"].as_str().unwrap_or_default().is_empty()&&a["isCurrentOneIdEnterprise"]!=true);
        let account=personal.filter(|a|a["uid"].as_str().is_some_and(|uid|expected.is_none_or(|id|slot(uid)==id))).next()
            .ok_or("Sign in to the originally linked personal account on the website, then try again")?;
        let id=slot(account["uid"].as_str().ok_or("Website returned no account ID")?);
        session.auth.as_mut().unwrap().uid=account["uid"].as_str().unwrap().into();
        let snapshot=query(&agent,&session,&id).map_err(Failure::message)?;
        if cancel.load(Ordering::Relaxed){return Err("WorkBuddy login canceled".into());}
        save_session(&id,&session)?;
        return Ok((id,snapshot));
    }
    Err("Website login timed out. Please reconnect".into())
}
#[cfg(test)]
mod tests{
    use super::*;
    use serde_json::json;
    #[test]
    fn login_url_must_match_official_origin_and_state(){
        let make=|url|json!({"code":0,"data":{"state":"test","authUrl":url}});
        assert!(login_request(make("https://www.codebuddy.cn/login?state=test&platform=daozai")).is_ok());
        for url in ["https://example.com/login?state=test","https://www.codebuddy.cn/login?state=other","http://www.codebuddy.cn/login?state=test"]{
            assert!(login_request(make(url)).is_err());
        }
    }
}
