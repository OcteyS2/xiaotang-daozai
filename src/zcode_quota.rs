use super::{Account, Snapshot, Window};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::{aead,digest};
use serde_json::Value;
use std::{io::Read,path::{Path,PathBuf}};

pub(crate) fn path()->Option<PathBuf>{
    Some(PathBuf::from(std::env::var_os("USERPROFILE")?).join(".zcode/v2/credentials.json"))
}
fn value(root:&Value,name:&str)->Result<String,String>{
    let raw=root[name].as_str().ok_or("Please sign in to a personal account in ZCode first")?;
    let Some(encoded)=raw.strip_prefix("enc:v1:") else{return Ok(raw.into());};
    let secret=std::env::var("ZCODE_CREDENTIAL_SECRET").ok().filter(|s|!s.trim().is_empty()).unwrap_or_else(||format!(
        "zcode-credential-fallback:win32:{}:{}",std::env::var("USERPROFILE").unwrap_or_default(),std::env::var("USERNAME").unwrap_or_default()));
    let key=digest::digest(&digest::SHA256,secret.trim().as_bytes());
    let error="Can’t read the ZCode sign-in for the current Windows user. Please sign in again in the app";
    let parts=encoded.split('.').map(|s|URL_SAFE_NO_PAD.decode(s)).collect::<Result<Vec<_>,_>>().map_err(|_|error)?;
    let [iv,tag,cipher]=parts.as_slice() else{return Err(error.into());};
    if tag.len()!=16{return Err(error.into());}
    let mut bytes=cipher.clone();bytes.extend_from_slice(tag);
    let key=aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM,key.as_ref()).map_err(|_|error)?);
    let plain=key.open_in_place(aead::Nonce::try_assume_unique_for_key(iv).map_err(|_|error)?,aead::Aad::empty(),&mut bytes).map_err(|_|error)?;
    String::from_utf8(plain.to_vec()).map_err(|_|error.into())
}
struct Login { family:String, slot:String, subject:String, jwt:String }
fn login(root:&Value)->Result<Login,String>{
    let family=value(root,"oauth:active_provider")?;
    if !matches!(family.as_str(),"zai"|"bigmodel"){return Err("This ZCode sign-in region isn’t supported yet".into());}
    let jwt=value(root,"zcodejwttoken")?;
    let payload=jwt.split('.').nth(1).and_then(|s|URL_SAFE_NO_PAD.decode(s).ok()).and_then(|s|serde_json::from_slice::<Value>(&s).ok()).ok_or("Invalid ZCode account ID")?;
    let id=payload["sub"].as_str().or_else(||payload["user_id"].as_str()).ok_or("ZCode returned no account ID")?;
    let slot=digest::digest(&digest::SHA256,format!("{family}/{id}").as_bytes()).as_ref().iter().map(|b|format!("{b:02x}")).collect();
    Ok(Login{family,slot,subject:id.to_owned(),jwt})
}
pub(crate) fn add(config:&mut super::Config,path:&Path,alias:&str)->Result<(),String>{
    let l=login(&super::login_json(path)?)?;
    if config.accounts.iter().any(|a|a.provider==super::Provider::ZCode&&a.slot.as_ref()==Some(&l.slot)){return Ok(());}
    config.accounts.push(Account{id:format!("ZCode-{}",l.slot),provider:super::Provider::ZCode,alias:if alias.trim().is_empty(){"智谱 ZCode".into()}else{alias.trim().into()},source:path.to_owned(),slot:Some(l.slot),enabled:true,pinned:false});Ok(())
}
fn read(agent:&ureq::Agent,url:&str,auth:&str,device_mid:Option<&str>)->Result<Value,String>{
    let mut request=agent.get(url).set("Authorization",auth).set("Accept","application/json");
    if let Some(mid)=device_mid{request=request.set("X-Device-Mid",mid);}
    let response=request.call().map_err(|e|match e{
        ureq::Error::Status(401|403,_)=>"ZCode authorization denied. Please confirm sign-in in the app".into(),
        ureq::Error::Status(code,_)=>format!("ZCode usage service returned {code}"),
        _=>"Can’t connect to the ZCode usage service".into(),
    })?;
    serde_json::from_reader(response.into_reader().take(1024*1024)).map_err(|_|"Invalid ZCode usage response".into())
}
fn success(v:&Value)->bool{ matches!(v["code"].as_i64(),Some(0|200)) || (v["code"].is_null()&&v["success"]==true) }
fn coding(v:&Value)->Result<Snapshot,String>{
    let mut snapshot=Snapshot{plan:"Coding Plan".into(),at:chrono::Utc::now().timestamp(),..Default::default()};
    for row in v["data"]["limits"].as_array().into_iter().flatten(){
        let Some(used)=super::number(&row["percentage"]).filter(|v|(0.0..=100.0).contains(v)) else{continue;};
        let multiplier=match row["unit"].as_i64(){Some(1)=>Some(86400),Some(3)=>Some(3600),Some(5)=>Some(60),Some(6)=>Some(604800),_=>None};
        let seconds=row["number"].as_i64().filter(|n|*n>0).zip(multiplier).and_then(|(n,m)|n.checked_mul(m));
        let kind=row["type"].as_str().unwrap_or_default();
        let label=if kind=="TIME_LIMIT"{"工具调用".into()}else{super::cadence(seconds)};
        let reset=row["nextResetTime"].as_i64().filter(|x|*x>0).map(|x|if x>10_000_000_000{x/1000}else{x});
        let window=Window{label,seconds,remaining:(100.0-used) as f32,reset};
        match kind{"TOKENS_LIMIT"|"CREDIT_LIMIT"=>snapshot.windows.push(window),"TIME_LIMIT"=>snapshot.extra.push(window),_=>{}}
    }
    if snapshot.windows.is_empty(){std::mem::swap(&mut snapshot.windows,&mut snapshot.extra);}
    if snapshot.windows.is_empty(){return Err("ZCode provided no plan ratio to display".into());}
    Ok(snapshot)
}
fn start(v:&Value)->Result<Snapshot,String>{
    let mut snapshot=Snapshot{plan:"Start Plan".into(),at:chrono::Utc::now().timestamp(),..Default::default()};
    let buckets=v["data"]["balances"].as_array().ok_or("ZCode response is missing trial quota details")?;
    if buckets.is_empty(){snapshot.plan="暂无体验额度".into();return Ok(snapshot);}
    for bucket in buckets{
        let Some(total)=super::number(&bucket["total_units"]).filter(|n|*n>0.0) else{continue;};
        let Some(left)=super::number(&bucket["remaining_units"]).filter(|n|*n>=0.0) else{continue;};
        let reset=super::number(&bucket["expires_at"]).map(|n|n as i64);
        if reset.is_some_and(|t|t<=snapshot.at){continue;}
        snapshot.windows.push(Window{label:bucket["show_name"].as_str().filter(|s|!s.is_empty()).unwrap_or("体验额度").into(),seconds:None,remaining:(left/total*100.0).clamp(0.0,100.0) as f32,reset});
    }
    if snapshot.windows.is_empty(){return Err("This ZCode account has no active trial quota".into());}
    Ok(snapshot)
}
pub(super) fn fetch(agent:&ureq::Agent,a:&Account)->Result<Snapshot,String>{
    let root=super::login_json(&a.source)?;let l=login(&root)?;
    if a.slot.as_ref()!=Some(&l.slot){return Err("ZCode switched accounts. Please re-detect the local account".into());}
    let key=format!("account-provider:coding-plan:account:{}-individual-coding-plan:account:{}:api-key",l.family,l.subject);
    let mut no_coding_plan=false;
    if root.get(&key).is_some(){
        let url=if l.family=="zai"{"https://api.z.ai/api/monitor/usage/quota/limit"}else{"https://bigmodel.cn/api/monitor/usage/quota/limit"};
        let v=read(agent,url,&value(&root,&key)?,None)?;
        if success(&v){return coding(&v);}
        let msg=v["msg"].as_str().or_else(||v["message"].as_str()).unwrap_or("").to_lowercase();
        if !(msg.contains("不存在coding plan")||msg.contains("不存在 coding plan")||msg.contains("no coding plan")){return Err(format!("ZCode coding plan query failed ({})",v["code"]));}
        no_coding_plan=true;
    }
    let mut url=url::Url::parse("https://zcode.z.ai/api/v1/zcode-plan/billing/balance").unwrap();
    // 客户端按版本发布运行时，官网使用 app_version 判断体验套餐能力。
    if let Some(runtime)=a.source.parent().map(|p|p.join("runtime/provider/windows-x86_64")){
        let versions=std::fs::read_dir(runtime).into_iter().flatten().flatten().filter_map(|dir|{
            let name=dir.file_name().into_string().ok()?;
            let parts=name.split('.').map(str::parse::<u32>).collect::<Result<Vec<_>,_>>().ok()?;
            (parts.len()==3).then_some((parts,name))
        });
        if let Some((_,v))=versions.max(){url.query_pairs_mut().append_pair("app_version",&v);}
    }
    let telemetry=a.source.with_file_name("telemetry-state.json");
    let device=super::login_json(&telemetry).ok();
    let mid=device.as_ref().and_then(|v|v["deviceMid"].as_str()).filter(|s|!s.is_empty() && s.bytes().all(|b|b.is_ascii_graphic())).ok_or("Please open ZCode and wait for it to initialize device info")?;
    let v=read(agent,url.as_str(),&format!("Bearer {}",l.jwt),Some(mid)).map_err(|e|if no_coding_plan{format!("No Coding Plan; trial plan query failed: {e}")}else{e})?;
    if !success(&v){return Err(format!("ZCode trial quota unavailable ({})",v["code"]));}
    start(&v)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn empty_entitlements_are_valid_but_missing_data_is_an_error(){
        assert!(start(&serde_json::json!({"data":{"balances":[]}})).unwrap().windows.is_empty());
        assert!(start(&serde_json::json!({"data":{}})).is_err());
    }
    #[test]fn zcode_distinguishes_used_percentage_and_remaining_units(){
        let c=coding(&serde_json::json!({"data":{"limits":[{"type":"TOKENS_LIMIT","percentage":31,"unit":3,"number":5,"nextResetTime":1900000000000i64}]}})).unwrap();
        assert_eq!(c.windows[0].remaining,69.0);assert_eq!(c.windows[0].reset,Some(1900000000));
        let s=start(&serde_json::json!({"data":{"balances":[{"total_units":"800","remaining_units":"200","available_units":"100"}]}})).unwrap();assert_eq!(s.windows[0].remaining,25.0);
    }
}
