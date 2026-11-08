//! Gemini 网页订阅额度，接口与字段映射参考 CodexBar 的 gemini-web 插件。
//! 浏览器连接只在用户点击时读取会话；会话由 Windows DPAPI 加密保存。
use super::{Account,Snapshot,Window};
use serde_json::{json,Value};
use std::{fs,io::Read,path::PathBuf,sync::mpsc,time::Duration};

const ENDPOINT:&str="https://gemini.google.com/_/BardChatUi/graphql/schemas/GEMINI_WEB_GRAPHQL/executeQuery";
const SIGNATURE:&str="2/dWbCvkVB0X16r9NPnxifeOAAqdCqMf0xeWvau9yPy7g=";
const LOGIN_ERROR:&str="Gemini sign-in expired, please reconnect your browser account";
const PARSE_ERROR:&str="Gemini quota API changed, can't read it for now";
const EMAIL_SCRIPT:&str=r#"JSON.stringify({email:(document.querySelector('a[aria-label*="@"]')?.getAttribute('aria-label')||'').match(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/i)?.[0]||''})"#;

pub fn parse(value:&Value)->Result<Snapshot,String>{
    let data=value.as_array().ok_or(PARSE_ERROR)?.iter().filter_map(|v|v.get("data")).find(|d|d.get("viewer").is_some()).ok_or(PARSE_ERROR)?;
    if data["viewer"].is_null(){return Err(LOGIN_ERROR.into());}
    let usage=data.pointer("/viewer/rpcs/getUsageInfo").ok_or(PARSE_ERROR)?;
    let pools=usage["pools"].as_array().ok_or(PARSE_ERROR)?;
    let windows=[("GXU_FIVE_HOURLY",18000,"5h"),("GXU_WEEKLY",604800,"Weekly")].into_iter().filter_map(|(kind,seconds,label)|{
        let pool=pools.iter().find(|p|p["poolType"]==kind)?;
        let used=pool["percentageUsed"].as_f64().filter(|n|n.is_finite() && *n>=0.0)?;
        Some(Window{label:label.into(),seconds:Some(seconds),remaining:(100.0-used*100.0).clamp(0.0,100.0) as f32,reset:super::reset(&pool["gxu"]["windowRefreshesAt"])})
    }).collect::<Vec<_>>();
    if windows.is_empty(){return Err("This Google account has no Gemini subscription quota yet".into());}
    Ok(Snapshot{credits:None,wallet:None,plan:if usage["subscriptionTier"]=="TIER1"{"AI Pro"}else{""}.into(),windows,extra:vec![],at:chrono::Utc::now().timestamp()})
}
fn decode(raw:&str)->Result<Snapshot,String>{
    let raw=raw.trim_start();let raw=raw.strip_prefix(")]}'").unwrap_or(raw).trim_start();
    if raw.starts_with('<'){return Err(LOGIN_ERROR.into());}
    parse(&serde_json::from_str::<Value>(raw).map_err(|_|PARSE_ERROR)?)
}
fn email(raw:&str)->Result<String,String>{
    let value=raw.trim().to_ascii_lowercase();
    if value.len()>254 || !value.is_ascii() || value.chars().any(char::is_whitespace) || value.split('@').count()!=2 || !value.split('@').all(|s|!s.is_empty()) {return Err("Please enter this Google account's full email address".into());}
    Ok(value)
}
fn slot(email:&str)->String{ring::digest::digest(&ring::digest::SHA256,email.as_bytes()).as_ref().iter().map(|b|format!("{b:02x}")).collect()}
fn cookie(raw:&str)->Result<String,String>{
    let raw=raw.trim();let raw=raw.strip_prefix("Cookie:").or_else(||raw.strip_prefix("cookie:")).unwrap_or(raw).trim();
    if raw.is_empty() || raw.len()>32768 || !raw.is_ascii() || raw.bytes().any(|b|b<32 || b==127){return Err("Please paste a valid Cookie request header".into());}
    if !raw.split(';').any(|part|part.trim().split_once('=').is_some_and(|(name,value)|matches!(name,"SID"|"__Secure-1PSID"|"__Secure-3PSID") && !value.is_empty())){return Err("Cookie has no Google sign-in session, please copy it again".into());}
    Ok(raw.into())
}
struct Session {email:String,cookie:String}
fn request_url(email:&str)->String{
    let mut url=url::Url::parse(ENDPOINT).unwrap();
    let variables=json!({"g1Request":{"entrypoints":["GEMINI_USAGE_SETTINGS_UPGRADE_BANNER"]},"workspaceRequest":{"recommendationsForSurfaceRequests":[]}}).to_string();
    url.query_pairs_mut().append_pair("query_signature",SIGNATURE).append_pair("variables",&variables).append_pair("authuser",email);url.into()
}
fn redirect_target(current:&str,location:&str)->Option<String>{
    let target=url::Url::parse(current).ok()?.join(location).ok()?;
    // Google 会把邮箱形式的 authuser 规范化为账号编号；只跟随同站的用量查询。
    (target.scheme()=="https" && target.host_str()==Some("gemini.google.com") && target.port_or_known_default()==Some(443) && target.username().is_empty() && target.password().is_none() && target.path().ends_with("/_/BardChatUi/graphql/schemas/GEMINI_WEB_GRAPHQL/executeQuery")).then(||target.into())
}
fn request(agent:&ureq::Agent,url:&str,session:&Session)->Result<Snapshot,(String,u64)>{
    let mut current=url.to_owned();
    for _ in 0..3{
    let response=agent.get(&current).set("Cookie",&session.cookie).set("Accept","application/json").set("Referer","https://gemini.google.com/usage").set("User-Agent","Mozilla/5.0 Daozai").call().map_err(|e|match e{
        ureq::Error::Status(401|403,_)=>(LOGIN_ERROR.into(),0),
        ureq::Error::Status(429,r)=>("Too many Gemini requests, will retry automatically later".into(),r.header("Retry-After").and_then(|s|s.parse().ok()).unwrap_or(180).clamp(30,3600)),
        ureq::Error::Status(n,_)=>(format!("Gemini quota service unavailable ({n})"),60),
        _=>("Can't connect to Gemini. Check your network or proxy".into(),30)
    })?;
    if (300..400).contains(&response.status()){
        current=response.header("Location").and_then(|to|redirect_target(&current,to)).ok_or_else(||(LOGIN_ERROR.into(),0))?;continue;
    }
    let mut body=String::new();response.into_reader().take(1024*1024).read_to_string(&mut body).map_err(|_|("Failed to read Gemini quota".into(),30))?;
    return decode(&body).map_err(|e|(e,60));
    }
    Err((LOGIN_ERROR.into(),0))
}
fn session_path(slot:&str)->Result<PathBuf,String>{
    if slot.len()!=64 || !slot.bytes().all(|b|b.is_ascii_hexdigit()){return Err(LOGIN_ERROR.into());}
    Ok(crate::model::config_dir().join("credentials").join(format!("gemini-{slot}.bin")))
}
#[cfg(windows)]
pub(super) fn protect(raw:&[u8],encrypt:bool)->Result<Vec<u8>,String>{
    use windows::Win32::{Foundation::{HLOCAL,LocalFree},Security::Cryptography::{CryptProtectData,CryptUnprotectData,CRYPT_INTEGER_BLOB,CRYPTPROTECT_UI_FORBIDDEN}};
    let input=CRYPT_INTEGER_BLOB{cbData:raw.len().try_into().map_err(|_|LOGIN_ERROR)?,pbData:raw.as_ptr().cast_mut()};let mut output=CRYPT_INTEGER_BLOB::default();
    unsafe{
        if encrypt{CryptProtectData(&input,None,None,None,None,CRYPTPROTECT_UI_FORBIDDEN,&mut output)}else{CryptUnprotectData(&input,None,None,None,None,CRYPTPROTECT_UI_FORBIDDEN,&mut output)}.map_err(|_|"Can't access the sign-in session saved securely by Windows")?;
        let bytes=std::slice::from_raw_parts(output.pbData,output.cbData as usize).to_vec();let _=LocalFree(HLOCAL(output.pbData.cast()));Ok(bytes)
    }
}
#[cfg(not(windows))]
pub(super) fn protect(_raw:&[u8],_encrypt:bool)->Result<Vec<u8>,String>{Err("Secure session storage isn't supported on this system yet".into())}
fn save(session:&Session)->Result<String,String>{
    let slot=slot(&session.email);let path=session_path(&slot)?;
    let raw=json!({"email":session.email,"cookie":session.cookie}).to_string();let encrypted=protect(raw.as_bytes(),true)?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|_|"Can't save Gemini sign-in")?;
    crate::model::atomic_write(&path,&encrypted).map_err(|_|"Can't save Gemini sign-in")?;Ok(slot)
}
fn load(account:&Account)->Result<Session,String>{
    let expected=account.slot.as_deref().ok_or(LOGIN_ERROR)?;let path=session_path(expected)?;
    let raw=fs::read(path).map_err(|_|LOGIN_ERROR)?;let raw=protect(&raw,false)?;
    let value:Value=serde_json::from_slice(&raw).map_err(|_|LOGIN_ERROR)?;
    let email=email(value["email"].as_str().ok_or(LOGIN_ERROR)?)?;
    if slot(&email)!=expected{return Err("Gemini account mismatch, please reconnect".into());}
    Ok(Session{email,cookie:cookie(value["cookie"].as_str().ok_or(LOGIN_ERROR)?)?})
}
pub fn forget(account:&Account){if let Some(path)=account.slot.as_deref().and_then(|s|session_path(s).ok()){let _=fs::remove_file(path);}}
pub fn fetch(agent:&ureq::Agent,account:&Account)->Result<Snapshot,(String,u64)>{
    let session=load(account).map_err(|e|(e,0))?;request(agent,&request_url(&session.email),&session)
}
pub struct Connected {pub slot:String,pub label:String,pub snapshot:Snapshot}
pub struct Connect {rx:mpsc::Receiver<Result<Connected,String>>}
impl Connect {
    pub fn start(alias:String,address:String,header:Option<String>,expected:Option<String>)->Self{
        let(tx,rx)=mpsc::channel();std::thread::spawn(move||{let result=(||{
            let session=if let Some(header)=header{Session{email:email(&address)?,cookie:cookie(&header)?}}else{browser_session(&address)?};
            if expected.as_deref().is_some_and(|s|s!=slot(&session.email)){return Err("The browser is on a different Google account. Switch back to the original account and reconnect".into());}
            let snapshot=request(&super::http_agent(),&request_url(&session.email),&session).map_err(|e|e.0)?;
            let slot=save(&session)?;let label=if alias.trim().is_empty(){format!("Gemini · {}",session.email)}else{alias.trim().into()};
            Ok(Connected{slot,label,snapshot})
        })();let _=tx.send(result);});Self{rx}
    }
    pub fn poll(&self)->Option<Result<Connected,String>>{self.rx.try_recv().ok()}
}
fn bridge(agent:&ureq::Agent,action:&str,args:Value)->Result<Value,String>{
    let body=json!({"action":action,"args":args,"session":"daozai-gemini"});
    let response=agent.post("http://127.0.0.1:10086/command").set("Content-Type","application/json").send_string(&body.to_string()).map_err(|_|"Kimi browser extension not connected; connect the extension, or use the Cookie connection below")?;
    let mut raw=String::new();response.into_reader().take(2*1024*1024).read_to_string(&mut raw).map_err(|_|"Can't read browser session")?;
    let value:Value=serde_json::from_str(&raw).map_err(|_|"Browser extension returned invalid data")?;
    if value["ok"]!=true{return Err("Can't read the Gemini tab. Check the browser extension connection".into());}
    value.get("data").cloned().ok_or_else(||"Can't read browser session".into())
}
fn browser_session(address:&str)->Result<Session,String>{
    let agent=ureq::AgentBuilder::new().timeout(Duration::from_secs(12)).redirects(0).build();
    let response=bridge(&agent,"list_browser_tabs",json!({}))?;
    let mut tabs=response["tabs"].as_array().ok_or("Can't read browser tabs")?.iter().filter(|t|t["url"].as_str().and_then(|s|url::Url::parse(s).ok()).is_some_and(|u|u.scheme()=="https" && u.host_str()==Some("gemini.google.com"))).collect::<Vec<_>>();
    if tabs.is_empty(){return Err("Open gemini.google.com/usage in your browser and sign in first, then connect this browser".into());}
    tabs.sort_by_key(|t|!t["active"].as_bool().unwrap_or(false));
    let target=if address.trim().is_empty(){None}else{Some(email(address)?)};
    if tabs.len()>1 && target.is_none() && tabs[0]["active"]!=true{return Err("Multiple Gemini tabs open. Activate the tab to connect, or enter the Google email".into());}
    for tab in tabs{
        bridge(&agent,"attach_tab",json!({"tabId":tab["tabId"]}))?;
        let result=bridge(&agent,"evaluate",json!({"code":EMAIL_SCRIPT}))?;
        let identity:Value=serde_json::from_str(result["value"].as_str().unwrap_or_default()).map_err(|_|"Can't identify the Gemini account. Open the usage page and retry")?;
        let current=email(identity["email"].as_str().unwrap_or_default()).map_err(|_|"Sign in to a Google account on the Gemini usage page and retry")?;
        if target.as_ref().is_some_and(|target|*target!=current){continue;}
        let result=bridge(&agent,"cdp",json!({"method":"Network.getCookies","params":{"urls":["https://gemini.google.com/"]}}))?;
        let cookies=result["cookies"].as_array().ok_or("Can't read Gemini sign-in session")?;
        let header=cookies.iter().filter(|c|matches!(c["domain"].as_str(),Some(".google.com"|"google.com"|".gemini.google.com"|"gemini.google.com"))).filter_map(|c|Some(format!("{}={}",c["name"].as_str()?,c["value"].as_str()?))).collect::<Vec<_>>().join("; ");
        return Ok(Session{email:current,cookie:cookie(&header)?});
    }
    Err("No Gemini tab found for this Google account. Open that account's usage page first".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(five:Value,week:Value)->Value{json!([{"data":{"viewer":{"rpcs":{"getUsageInfo":{"subscriptionTier":"TIER1","pools":[{"poolType":"GXU_FIVE_HOURLY","percentageUsed":five,"gxu":{"windowRefreshesAt":"2030-01-01T08:24:11Z"}},{"poolType":"GXU_WEEKLY","percentageUsed":week,"gxu":{"windowRefreshesAt":"2030-01-04T03:24:11Z"}}]}}}},"errors":[{"message":"unrelated upsell"}]}])}
    #[test] fn gemini_fractional_usage_and_real_resets(){let s=parse(&response(json!(0.24),json!(0.01786011))).unwrap();assert_eq!(s.plan,"AI Pro");assert_eq!(s.rings().len(),2);assert_eq!(s.windows[0].remaining,76.0);assert!((s.windows[1].remaining-98.21399).abs()<0.001);assert_eq!(s.windows[0].reset,Some(1893486251));assert_eq!(s.windows[0].seconds,Some(18000));assert_eq!(s.windows[1].seconds,Some(604800));}
    #[test] fn gemini_zero_exhaustion_and_missing_windows(){let s=parse(&response(json!(0),json!(1))).unwrap();assert_eq!(s.windows[0].remaining,100.0);assert_eq!(s.windows[1].remaining,0.0);for bad in [Value::Null,json!(-1),json!("0.25"),json!("NaN")]{let s=parse(&response(bad.clone(),json!(0.5))).unwrap();assert_eq!(s.rings().len(),1);assert_eq!(s.rings()[0].label,"本周");assert!(parse(&response(bad.clone(),bad)).is_err());}assert_eq!(parse(&response(json!(1.2),Value::Null)).unwrap().windows[0].remaining,0.0);}
    #[test] fn gemini_signed_out_or_changed_queries_are_not_full_quota(){for raw in [r#"[{"data":{"viewer":null}}]"#,r#"[{"data":{}}]"#,"{}","[]",r#"[{"data":{"viewer":{"rpcs":{"getUsageInfo":{"pools":[]}}}}}]"#]{assert!(decode(raw).is_err());}let raw=format!(")]}}'\n{}",response(json!(0.3),json!(0.2)));assert_eq!(decode(&raw).unwrap().windows[0].remaining,70.0);assert!(decode("<html>private</html>").unwrap_err().contains("登录"));}
    #[test] fn gemini_unknown_plan_and_reset_stay_unknown(){let mut v=response(json!(0.1),Value::Null);let usage=&mut v[0]["data"]["viewer"]["rpcs"]["getUsageInfo"];usage["subscriptionTier"]=json!("NEW_TIER");usage["pools"][0]["gxu"]["windowRefreshesAt"]=json!("unknown");let s=parse(&v).unwrap();assert!(s.plan.is_empty());assert!(s.windows[0].reset.is_none());}
    #[test] fn gemini_account_selection_and_cookie_validation(){let e=email(" Personal@Example.COM ").unwrap();assert_eq!(e,"personal@example.com");let u=url::Url::parse(&request_url(&e)).unwrap();let pairs=u.query_pairs().collect::<std::collections::HashMap<_,_>>();assert_eq!(pairs["authuser"],e);assert_eq!(pairs["query_signature"],SIGNATURE);assert_eq!(u.host_str(),Some("gemini.google.com"));assert_ne!(slot(&e),slot("work@example.com"));for bad in ["", "test", "a@b\r\nInjected: yes"]{assert!(email(bad).is_err());}assert_eq!(cookie("Cookie: SID=test; SAPISID=other").unwrap(),"SID=test; SAPISID=other");for bad in ["", "SID=test\r\nX: bad", "foo=bar"]{assert!(cookie(bad).is_err());}assert!(session_path("../../other").is_err());}
    #[test] fn gemini_account_redirects_never_forward_cookies_to_another_site(){let url=request_url("personal@example.com");let target=redirect_target(&url,"?authuser=0").unwrap();assert_eq!(url::Url::parse(&target).unwrap().query(),Some("authuser=0"));for to in ["https://accounts.google.com/signin","https://gemini.google.com.evil.invalid/","http://gemini.google.com/","https://gemini.google.com:444/","https://user@gemini.google.com/","/app"]{assert!(redirect_target(&url,to).is_none());}}
    #[cfg(windows)] #[test] fn gemini_windows_storage_is_encrypted_and_roundtrips(){let raw=b"{\"email\":\"synthetic@example.com\",\"cookie\":\"SID=synthetic-secret\"}";let sealed=protect(raw,true).unwrap();assert!(!sealed.windows(16).any(|w|w==b"synthetic-secret"));assert_eq!(protect(&sealed,false).unwrap(),raw);let mut corrupt=sealed;corrupt[24]^=1;assert!(protect(&corrupt,false).is_err());}
    #[test] fn gemini_read_only_request_does_not_echo_private_errors(){
        use std::net::TcpListener;let server=TcpListener::bind("127.0.0.1:0").unwrap();let url=format!("http://{}/quota",server.local_addr().unwrap());
        let task=std::thread::spawn(move||{let(mut stream,_)=server.accept().unwrap();let mut bytes=[0u8;4096];let n=stream.read(&mut bytes).unwrap();let raw=String::from_utf8_lossy(&bytes[..n]);assert!(raw.starts_with("GET /quota HTTP/1.1"));assert!(raw.to_ascii_lowercase().contains("cookie: sid=synthetic"));use std::io::Write;stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 14\r\nConnection: close\r\n\r\nprivate-secret").unwrap();});
        let session=Session{email:"synthetic@example.com".into(),cookie:"SID=synthetic".into()};let error=request(&ureq::AgentBuilder::new().redirects(0).build(),&url,&session).unwrap_err();assert!(error.0.contains("重新连接"));assert!(!error.0.contains("private-secret"));task.join().unwrap();
    }
    #[test] #[ignore="刷新已绑定的 Gemini 浏览器登录并安全保存"]
    fn gemini_browser_reconnect_local(){
        let config=super::super::Config::load();
        let a=config.accounts.iter().find(|a|a.enabled && a.provider==super::super::Provider::Gemini).unwrap();
        let current=load(a).unwrap();
        let session=browser_session(&current.email).unwrap();
        let snapshot=request(&super::super::http_agent(),&request_url(&session.email),&session).unwrap();
        assert!(!snapshot.windows.is_empty());assert_eq!(a.slot.as_ref(),Some(&slot(&session.email)));save(&session).unwrap();
    }
    #[test] #[ignore="按需验证已登录浏览器的真实 Gemini 额度"] fn gemini_browser_live_read_only(){
        let session=browser_session("").unwrap();let s=request(&super::super::http_agent(),&request_url(&session.email),&session).unwrap();println!("Gemini 真实额度：{} 个周期，剩余 {:?}",s.windows.len(),s.windows.iter().map(|w|w.remaining).collect::<Vec<_>>());assert!(!s.windows.is_empty());
    }
}
