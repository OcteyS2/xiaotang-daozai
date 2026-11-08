//! 日历：飞书 / 钉钉 / iCloud / 自定义 CalDAV 与企业微信 Exchange ActiveSync 只读接入。
//!
//! `PROPFIND` 找日历（uuid），`REPORT calendar-query` 分别查询提醒窗口与所选月份，
//! 企业微信使用 FolderSync / Sync；复用 VEVENT 解析与重复日程展开。认证使用 Basic Auth，
//! 没有 OAuth、没有回调，用户在客户端自己生成账号密码即可。
//!
//! 硬约束：只读；凭据只进 Windows 凭据管理器（`keyring`，和邮件同一套库），
//! 配置文件里只有平台、用户名、日历地址；不落盘正文、不进日志。

mod exchange;

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use chrono::{Datelike, NaiveDate};

pub const SERVICE: &str = "Daozai.Calendar";
/// 开启自动同步时每 5 分钟更新一次。
const SYNC_SECS: u64 = 300;
/// 只取未来 24 小时。
const WINDOW_SECS: i64 = 24 * 3600;

// ---------------------------------------------------------------- 平台

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Provider {
    #[default]
    Feishu,
    DingTalk,
    WeCom,
    /// 苹果 iCloud 日历：Apple ID + App 专用密码。
    ICloud,
    /// 任意 CalDAV 服务（Nextcloud、群晖、Fastmail、Radicale 等），地址由用户填写。
    Custom,
    /// 自建或第三方 Exchange（ActiveSync），地址由用户填写；与企业微信共用同一套只读客户端。
    Exchange,
}

impl Provider {
    pub const ALL: [Provider; 6] = [Provider::Feishu, Provider::DingTalk, Provider::WeCom, Provider::ICloud, Provider::Custom, Provider::Exchange];

    pub fn key(self) -> &'static str {
        match self {
            Provider::Feishu => "feishu",
            Provider::DingTalk => "dingtalk",
            Provider::WeCom => "wecom",
            Provider::ICloud => "icloud",
            Provider::Custom => "custom",
            Provider::Exchange => "exchange",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Feishu => "Feishu",
            Provider::DingTalk => "DingTalk",
            Provider::WeCom => "WeCom",
            Provider::ICloud => "iCloud",
            Provider::Custom => "CalDAV",
            Provider::Exchange => "Exchange",
        }
    }

    /// 平台同步入口，界面上只读显示。
    pub fn server(self) -> &'static str {
        match self {
            Provider::Feishu => "https://caldav.feishu.cn/calendars/",
            Provider::DingTalk => "https://calendar.dingtalk.com",
            Provider::WeCom => "https://wecom.work",
            Provider::ICloud => "https://caldav.icloud.com/",
            Provider::Custom | Provider::Exchange => "",
        }
    }

    /// 自定义 CalDAV、Exchange 需要用户填写服务器地址。
    pub fn custom_server(self) -> bool { matches!(self, Provider::Custom | Provider::Exchange) }

    /// 走 Exchange ActiveSync 协议，而不是 CalDAV。
    pub fn exchange(self) -> bool { matches!(self, Provider::WeCom | Provider::Exchange) }

    /// 服务器地址输入框的示例。
    pub fn server_hint(self) -> &'static str {
        if self == Provider::Exchange { "mail.example.com" } else { "https://example.com/remote.php/dav" }
    }

    /// 一个账号下有多个日历（iCloud 的「家庭」「工作」等）时全部同步。
    pub fn all_calendars(self) -> bool { matches!(self, Provider::ICloud | Provider::Custom) }

    /// 「在哪拿？」三步引导，直接对应客户端菜单。
    pub fn guide(self) -> [&'static str; 3] {
        match self {
            Provider::Feishu => [
                "Desktop app: avatar (top right) → Settings",
                "Calendar → CalDAV Sync Settings → choose 'Other devices'",
                "Click Generate, then copy the username and password into the fields above",
            ],
            Provider::DingTalk => [
                "Open DingTalk → Calendar",
                "Calendar Settings → Sync to other calendars / CalDAV",
                "Generate an account, then copy the username and password into the fields above",
            ],
            Provider::WeCom => [
                "Mobile app: Workbench → Schedule",
                "Tap the '≡' menu (top right) → Schedule Settings",
                "Sync to other calendars (Exchange) → copy the full username and the generated password",
            ],
            Provider::ICloud => [
                "Open account.apple.com in a browser and sign in to your Apple Account",
                "Sign-In and Security → App-Specific Passwords → generate one (you can name it 'Daozai')",
                "Username: your Apple ID email; password: the app-specific password you just generated",
            ],
            Provider::Custom => [
                "Server: the calendar service’s CalDAV URL, e.g. https://example.com/remote.php/dav",
                "Use your account on that service; if it supports app passwords, generate a separate one",
                "Read-only sync; Daozai discovers all calendars in the account automatically",
            ],
            Provider::Exchange => [
                "Server: your Exchange mail server address, e.g. mail.example.com (ask your IT team)",
                "Username is usually your email, or 'DOMAIN\\username'; password is your email password",
                "Read-only calendar sync; Microsoft 365 / Outlook.com no longer allow password sign-in, so they can’t be connected yet",
            ],
        }
    }

    pub fn parse(key: &str) -> Self {
        match key {
            "dingtalk" => Provider::DingTalk,
            "wecom" => Provider::WeCom,
            "icloud" => Provider::ICloud,
            "custom" => Provider::Custom,
            "exchange" => Provider::Exchange,
            _ => Provider::Feishu,
        }
    }
}

// ---------------------------------------------------------------- 配置

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub provider: Provider,
    pub username: String,
    /// 发现到的日历集合地址；`None` 表示还没发现。
    pub href: Option<String>,
    /// 服务器地址覆盖（只在录制/测试里用）。
    pub server: Option<String>,
}

impl Config {
    pub fn base(&self) -> String {
        self.server.clone().unwrap_or_else(|| self.provider.server().to_string())
    }
}

/// 自定义 CalDAV 地址：去掉空白，补全 https://；只接受 https（本机回环地址允许 http）。
pub fn normalize_server(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() { return None; }
    let raw = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let raw = raw.replacen("webcal://", "https://", 1).replacen("caldav://", "https://", 1).replacen("caldavs://", "https://", 1);
    let url = url::Url::parse(&raw).ok()?;
    let local = url.host_str().is_some_and(|host| host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()));
    if url.host_str().is_none_or(str::is_empty) || !url.username().is_empty() || url.password().is_some() { return None; }
    if !(url.scheme() == "https" || url.scheme() == "http" && local) { return None; }
    Some(url.to_string())
}

fn config_path() -> std::path::PathBuf {
    crate::model::config_dir().join("calendar.json")
}

pub fn load_config() -> Option<Config> {
    decode_config(&std::fs::read(config_path()).ok()?)
}

pub fn load_configs() -> Vec<Config> {
    let Ok(raw) = std::fs::read(config_path()) else { return vec![]; };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&raw) else { return vec![]; };
    if let Some(accounts) = json.get("accounts").and_then(|v| v.as_array()) {
        accounts.iter().filter_map(|v| decode_config(v.to_string().as_bytes())).collect()
    } else {
        decode_config(&raw).into_iter().collect()
    }
}

pub fn selected_provider() -> Provider {
    std::fs::read(config_path()).ok()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .and_then(|json| json.get("active").or_else(|| json.get("provider")).and_then(|v| v.as_str()).map(Provider::parse))
        .unwrap_or_default()
}

/// 纯解析：只有平台、用户名、日历地址，没有任何密码字段。
pub fn decode_config(raw: &[u8]) -> Option<Config> {
    let json: serde_json::Value = serde_json::from_slice(raw).ok()?;
    if let Some(accounts) = json.get("accounts").and_then(|v| v.as_array()) {
        let active = json.get("active")?.as_str()?;
        return accounts.iter().filter_map(|v| decode_config(v.to_string().as_bytes()))
            .find(|cfg| cfg.provider.key() == active);
    }
    let username = json.get("username")?.as_str()?.trim().to_string();
    if username.is_empty() {
        return None;
    }
    Some(Config {
        provider: Provider::parse(json.get("provider").and_then(|p| p.as_str()).unwrap_or("feishu")),
        username,
        href: json.get("href").and_then(|h| h.as_str()).map(|s| s.to_string()),
        server: json.get("server").and_then(|h| h.as_str()).map(|s| s.to_string()),
    })
}

pub fn encode_config(cfg: &Config) -> Vec<u8> {
    let json = serde_json::json!({
        "provider": cfg.provider.key(),
        "username": cfg.username,
        "href": cfg.href,
        "server": cfg.server,
    });
    serde_json::to_vec(&json).unwrap_or_default()
}

pub fn save_config(cfg: &Config) -> Result<(), String> {
    let mut accounts = load_configs();
    if let Some(saved) = accounts.iter_mut().find(|saved| saved.provider == cfg.provider) {
        *saved = cfg.clone();
    } else {
        accounts.push(cfg.clone());
    }
    save_configs(&accounts, cfg.provider)
}

fn save_configs(accounts: &[Config], active: Provider) -> Result<(), String> {
    let accounts: Vec<serde_json::Value> = accounts.iter()
        .map(|cfg| serde_json::from_slice(&encode_config(cfg)).expect("calendar config should serialize")).collect();
    let data = serde_json::to_vec(&serde_json::json!({"active": active.key(), "accounts": accounts}))
        .map_err(|e| format!("Failed to serialize config: {e}"))?;
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create config folder: {e}"))?;
    }
    crate::model::atomic_write(&path, &data).map_err(|e| format!("Failed to write config: {e}"))
}

pub fn select_provider(provider: Provider) -> Result<(), String> {
    save_configs(&load_configs(), provider)
}

/// 新连接完整保存后才移除旧凭据；配置保存失败时恢复原凭据。
pub fn save_connection(cfg: &Config, password: &str, old: Option<&Config>) -> Result<(), String> {
    let saved = load_configs().into_iter().find(|saved| saved.provider == cfg.provider);
    let old = saved.as_ref().or(old.filter(|old| old.provider == cfg.provider));
    let same_account = old.is_some_and(|old|
        old.provider == cfg.provider && old.username.eq_ignore_ascii_case(&cfg.username));
    let previous_password = secrets::load(cfg).ok();
    secrets::save(cfg, password)?;
    if let Err(error) = save_config(cfg) {
        if let Some(password) = previous_password {
            if let Err(restore) = secrets::save(cfg, &password) {
                return Err(format!("{error}; failed to restore previous credentials: {restore}"));
            }
        } else {
            secrets::delete(cfg);
        }
        return Err(error);
    }
    if let Some(old) = old.filter(|_| !same_account) {
        secrets::delete(old);
    }
    Ok(())
}

pub fn forget_config() -> Result<(), String> {
    let Some(active) = load_config() else { return Ok(()); };
    let mut accounts = load_configs();
    accounts.retain(|cfg| cfg.provider != active.provider);
    save_configs(&accounts, active.provider)
}

// ---------------------------------------------------------------- 凭据

#[cfg(windows)]
pub mod secrets {
    /// 凭据只进 Windows 凭据管理器；键是「平台:用户名」，值是密码。
    pub fn save(cfg: &super::Config, password: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(super::SERVICE, &name(cfg))
            .map_err(|e| format!("Credential Manager unavailable: {e}"))?;
        entry.set_password(password).map_err(|e| format!("Failed to save password: {e}"))
    }

    pub fn load(cfg: &super::Config) -> Result<String, String> {
        let entry = keyring::Entry::new(super::SERVICE, &name(cfg))
            .map_err(|e| format!("Credential Manager unavailable: {e}"))?;
        entry.get_password().map_err(|_| "No saved password".to_string())
    }

    pub fn delete(cfg: &super::Config) {
        if let Ok(entry) = keyring::Entry::new(super::SERVICE, &name(cfg)) {
            let _ = entry.delete_credential();
        }
    }

    fn name(cfg: &super::Config) -> String {
        format!("{}:{}", cfg.provider.key(), cfg.username.trim().to_ascii_lowercase())
    }
}

#[cfg(not(windows))]
pub mod secrets {
    pub fn save(_cfg: &super::Config, _password: &str) -> Result<(), String> {
        Err("Credential Manager is only supported on Windows".into())
    }
    pub fn load(_cfg: &super::Config) -> Result<String, String> {
        Err("Credential Manager is only supported on Windows".into())
    }
    pub fn delete(_cfg: &super::Config) {}
}

// ---------------------------------------------------------------- 粘贴拆分

/// 从整段粘贴里拆出用户名和密码。只认带标签的文本（用户名/账号/邮箱、
/// 密码/口令），标签之间用冒号、等号或空格分隔都行；拆不出就返回 `None`，
/// 不靠位置猜。
pub fn parse_paste(text: &str) -> Option<(String, String)> {
    let flat = text
        .replace(['\r', '\n', '\t'], " ")
        .replace('：', ":")
        .replace('＝', "=");
    let lower = flat.to_ascii_lowercase();
    let user_labels = ["用户名", "帐号", "账号", "邮箱", "账户", "username", "user name", "account", "email", "login"];
    let pass_labels = ["密码", "口令", "password", "pass word", "pwd", "pass"];
    let skip_labels = ["服务器地址", "服务器", "地址", "server", "url"];

    let mut marks: Vec<(usize, usize, u8)> = Vec::new(); // (start, end, 0=user 1=pass 2=skip)
    for (kind, labels) in [(0u8, &user_labels[..]), (1u8, &pass_labels[..]), (2u8, &skip_labels[..])] {
        for label in labels {
            let mut from = 0;
            while let Some(off) = lower[from..].find(label) {
                let start = from + off;
                marks.push((start, start + label.len(), kind));
                from = start + label.len();
            }
        }
    }
    if marks.is_empty() {
        return None;
    }
    marks.sort_by_key(|m| m.0);
    // 同一位置被多个标签命中时保留最长的（"用户名" 优于 "用户" 之类）。
    let mut picked: Vec<(usize, usize, u8)> = Vec::new();
    for m in marks {
        if let Some(last) = picked.last_mut() {
            if m.0 < last.1 {
                if m.1 - m.0 > last.1 - last.0 {
                    *last = m;
                }
                continue;
            }
        }
        picked.push(m);
    }

    let mut user = None;
    let mut pass = None;
    for (i, (_, end, kind)) in picked.iter().enumerate() {
        if *kind == 2 {
            continue;
        }
        let stop = picked.get(i + 1).map(|n| n.0).unwrap_or(flat.len());
        let value = clean(&flat[*end..stop]);
        if value.is_empty() {
            continue;
        }
        match kind {
            0 if user.is_none() => user = Some(value),
            1 if pass.is_none() => pass = Some(value),
            _ => {}
        }
    }
    match (user, pass) {
        (Some(u), Some(p)) if plausible(&u) && plausible(&p) => Some((u, p)),
        _ => None,
    }
}

fn clean(v: &str) -> String {
    v.trim().trim_matches(|c: char| c == ':' || c == '=' || c == '，' || c == ',').trim().to_string()
}

fn plausible(v: &str) -> bool {
    !v.is_empty() && v.chars().count() <= 200
}

// ---------------------------------------------------------------- HTTP

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub status: Option<u16>,
    pub message: String,
    request_url: Option<String>,
}

impl Failure {
    fn status(provider: Provider, code: u16) -> Self {
        let message = match code {
            401 if provider.custom_server() => "Account verification failed (401). Check your username and password".to_string(),
            401 => format!("Account verification failed (401). Check your username and the app password from {}", provider.label()),
            403 => "Calendar server denied access (403). Check the sync URL, app password and calendar permissions".to_string(),
            404 => "Wrong server address (404)".to_string(),
            405 => "This address doesn’t support calendar sync (405). Check the CalDAV URL from your provider".to_string(),
            407 => "Network requires proxy authentication (407)".to_string(),
            429 => "Too many requests (429). Try again later".to_string(),
            500..=599 => format!("Server error ({code}). Try again later"),
            _ => format!("Server returned {code}"),
        };
        Self { status: Some(code), message, request_url: None }
    }

    fn at_url(mut self, url: &url::Url) -> Self {
        self.request_url = Some(url.to_string());
        self
    }

    fn at_stage(mut self, stage: &str) -> Self {
        if self.status != Some(401) { self.message = format!("{stage} failed, {}", self.message); }
        self
    }

    fn transport(_detail: &str) -> Self {
        Self { status: None, message: "Can’t reach the server. Check your network".into(), request_url: None }
    }

    fn text(message: impl Into<String>) -> Self {
        Self { status: None, message: message.into(), request_url: None }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(20))
        .timeout_write(Duration::from_secs(20))
        .timeout(Duration::from_secs(30))
        .redirects(0)
        .user_agent(concat!("Daozai/",env!("CARGO_PKG_VERSION")))
        .build()
}

/// `Authorization: Basic base64(用户名:密码)`。
pub fn basic_auth(user: &str, password: &str) -> String {
    use base64::Engine;
    format!("Basic {}",base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}")))
}

fn dav(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    depth: &str,
    body: &str,
    cfg: &Config,
    password: &str,
) -> Result<String, Failure> {
    dav_response(agent,method,url,depth,body,cfg,password).map(|(xml,_)|xml)
}

/// 返回最终地址，让发现流程按重定向后的地址解析相对路径。
fn dav_response(
    agent: &ureq::Agent, method: &str, url: &str, depth: &str,
    body: &str, cfg: &Config, password: &str,
) -> Result<(String,String), Failure> {
    let base=url::Url::parse(&cfg.base()).map_err(|_|Failure::text("Invalid calendar server address"))?;
    let mut target=url::Url::parse(url).map_err(|_|Failure::text("Invalid calendar URL"))?;
    for attempt in 0..4 {
        let local=base.host_str().is_some_and(|host|host.parse::<std::net::IpAddr>().is_ok_and(|ip|ip.is_loopback()));
        if !same_service(cfg.provider,&target,&base) || !target.username().is_empty() || target.password().is_some()
            || !(target.scheme()=="https" || target.scheme()=="http" && local) {
            return Err(Failure::text("Calendar returned an untrusted address. Please reconnect"));
        }
        let res = agent
        .request(method, target.as_str())
        .set("Authorization", &basic_auth(&cfg.username, password))
        .set("Depth", depth)
        .set("Content-Type", "application/xml; charset=utf-8")
        .send_string(body);
        match res {
            Ok(r) if matches!(r.status(),301|302|303|307|308) && attempt<3 => {
                target=target.join(r.header("Location").ok_or_else(||Failure::text("Calendar redirect is missing an address"))?)
                    .map_err(|_|Failure::text("Invalid calendar redirect address"))?;
            }
            Ok(r) if r.status()>=300=>return Err(Failure::status(cfg.provider,r.status()).at_url(&target)),
            Ok(r)=>return r.into_string().map(|xml|(xml,target.to_string())).map_err(|e|Failure::transport(&e.to_string())),
            Err(ureq::Error::Status(code,_))=>return Err(Failure::status(cfg.provider,code).at_url(&target)),
            Err(e)=>return Err(Failure::transport(&e.to_string())),
        }
    }
    Err(Failure::text("Too many calendar redirects"))
}

/// iCloud 的日历实际放在 `pNN-caldav.icloud.com` 上，发现过程会从入口跳过去；
/// 其他平台只信任同源地址。
fn same_service(provider:Provider,target:&url::Url,base:&url::Url)->bool {
    if target.origin()==base.origin() {return true;}
    provider==Provider::ICloud && target.scheme()=="https" && target.port().is_none()
        && target.host_str().is_some_and(|host|host.ends_with(".icloud.com"))
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:resourcetype/><d:displayname/><d:current-user-principal/><c:calendar-home-set/><c:supported-calendar-component-set/></d:prop>
</d:propfind>"#;

fn calendar_data_request(_provider: Provider, start: &str, end: &str) -> String {
    format!(r#"<c:calendar-data><c:expand start="{start}" end="{end}"/></c:calendar-data>"#)
}

fn report_body(provider: Provider, start: &str, end: &str) -> String {
    let data=calendar_data_request(provider,start,end);
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/>{data}</d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">
    <c:time-range start="{start}" end="{end}"/>
  </c:comp-filter></c:comp-filter></c:filter>
</c:calendar-query>"#
    )
}

/// 一次拉取的结果。
pub struct Fetched {
    pub href: String,
    pub events: Vec<crate::mail::Meeting>,

}

/// 发现日历 → 查询滚动的 24 小时时间窗口 → 解析。
pub fn fetch(cfg: &Config, password: &str) -> Result<Fetched, Failure> {
    let now = chrono::Utc::now();
    fetch_range(cfg, password, now - chrono::Duration::minutes(1), now + chrono::Duration::seconds(WINDOW_SECS))
}

pub fn month_start(date: NaiveDate) -> NaiveDate { date.with_day(1).unwrap() }

pub fn month_grid(date: NaiveDate) -> [NaiveDate; 42] {
    let first = month_start(date);
    let start = first - chrono::Duration::days(first.weekday().num_days_from_monday() as i64);
    std::array::from_fn(|i| start + chrono::Duration::days(i as i64))
}

/// 按本地月份查询，结束时间为下月一日零点，不截成未来 24 小时。
pub fn fetch_month(cfg: &Config, password: &str, month: NaiveDate) -> Result<Fetched, Failure> {
    let first = month_start(month);
    let end = first.checked_add_months(chrono::Months::new(1)).unwrap();
    let utc = |date: NaiveDate| date.and_hms_opt(0, 0, 0).unwrap().and_local_timezone(chrono::Local).earliest().unwrap().with_timezone(&chrono::Utc);
    fetch_range(cfg, password, utc(first), utc(end))
}

fn fetch_range(cfg: &Config, password: &str, lo: chrono::DateTime<chrono::Utc>, hi: chrono::DateTime<chrono::Utc>) -> Result<Fetched, Failure> {
    if cfg.provider.exchange() { return exchange::fetch(cfg, password, lo, hi); }
    let agent = agent();
    // 每次都发现全部日历：已保存的地址可能指向空日历，会遮蔽同一账号下的其他日历。
    // 逐个查询，单个日历失败不影响其他日历，全部失败才报错。
    let calendars=discover_calendars(&agent,cfg,password).map_err(|e|e.at_stage("Finding calendars"))?;
    let href=calendars.first().cloned().ok_or_else(||Failure::text("No readable calendar found"))?;
    let mut events=Vec::new();
    let mut first_error=None;
    let mut ok=false;
    for collection in &calendars {
        match fetch_collection(&agent,cfg,password,collection,lo,hi) {
            Ok(found)=>{ok=true;events.extend(found.events);}
            Err(error)=>{first_error.get_or_insert(error.at_stage("Reading events"));}
        }
    }
    if !ok {return Err(first_error.unwrap_or_else(||Failure::text("No calendar found")));}
    events.sort_by_key(|m|m.start);
    let mut seen=std::collections::HashSet::new();events.retain(|m|seen.insert(dedupe_key(m)));
    Ok(Fetched{href,events})
}

fn fetch_collection(agent:&ureq::Agent,cfg:&Config,password:&str,href:&str,lo:chrono::DateTime<chrono::Utc>,hi:chrono::DateTime<chrono::Utc>)->Result<Fetched,Failure>{
    let href=href.to_owned();
    let start = lo.format("%Y%m%dT%H%M%SZ").to_string();
    let end = hi.format("%Y%m%dT%H%M%SZ").to_string();
    let mut xml = dav(&agent, "REPORT", &href, "1", &report_body(cfg.provider, &start, &end), cfg, password)?;
    if report_status_error(&xml).is_some() || (calendar_data(&xml).is_empty() && !responses(&xml).is_empty()) {
        // 有的服务只列资源，或在查询结果中附带集合本身；集合不能当成日程读取。
        if let Some(code @ (401|403))=report_status_error(&xml){return Err(Failure::status(cfg.provider,code));}
        let mut refs=report_resource_refs(&xml,&href)?;
        if refs.is_empty() {
            let listing=dav(&agent,"PROPFIND",&href,"1",PROPFIND_BODY,cfg,password)?;
            if let Some(code @ (401|403))=report_status_error(&listing){return Err(Failure::status(cfg.provider,code));}
            refs=report_resource_refs(&listing,&href)?;
            if refs.is_empty() {
                if let Some(code)=report_status_error(&xml){return Err(Failure::status(cfg.provider,code));}
                return Ok(Fetched{href,events:vec![]});
            }
        }
        let data=calendar_data_request(cfg.provider,&start,&end);
        let body=format!(r#"<c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/>{data}</d:prop>{refs}</c:calendar-multiget>"#);
        xml=dav(&agent,"REPORT",&href,"1",&body,cfg,password)?;
        if calendar_data(&xml).is_empty() && report_status_error(&xml).is_none() {
            return Err(Failure::text("Server returned no event data. Check calendar read permissions"));
        }
    }
    if let Some(code)=report_status_error(&xml){return Err(Failure::status(cfg.provider,code));}
    let events=events_in_range(&xml,lo,hi)?;
    Ok(Fetched { href, events })
}

fn report_resource_refs(xml:&str,href:&str)->Result<String,Failure> {
    let base=url::Url::parse(href).map_err(|_|Failure::text("Invalid calendar URL"))?;
    let mut refs=String::new();
    for (resource,is_calendar,_) in responses(xml) {
        let target=base.join(&resource).map_err(|_|Failure::text("Invalid event URL"))?;
        if target.origin()!=base.origin() || !target.username().is_empty() || target.password().is_some() {
            return Err(Failure::text("Calendar returned an untrusted address. Please reconnect"));
        }
        if is_calendar || target.path().trim_end_matches('/')==base.path().trim_end_matches('/') {continue;}
        let escaped=target[url::Position::BeforePath..url::Position::AfterQuery].replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;");
        refs.push_str(&format!("<d:href>{escaped}</d:href>"));
    }
    Ok(refs)
}

fn discover_calendars(agent: &ureq::Agent, cfg: &Config, password: &str) -> Result<Vec<String>, Failure> {
    let base = ensure_slash(cfg.base().trim());
    let root=path_of(&base)=="/";
    if let Some(calendar)=discover_at(agent,cfg,password,&base,"1")? {
        return Ok(calendar);
    }
    if root {
        let well_known=join_url(&base,".well-known/caldav");
        if let Some(calendar)=discover_at(agent,cfg,password,&well_known,"0")? {
            return Ok(calendar);
        }
        if let Some(calendar)=discover_at(agent,cfg,password,&join_url(&base,"calendars/"),"1")? {
            return Ok(calendar);
        }
    }
    Err(Failure::text("No calendar found. Make sure this account can sync events in its own app"))
}

fn discover_at(agent:&ureq::Agent,cfg:&Config,password:&str,start:&str,depth:&str)->Result<Option<Vec<String>>,Failure>{
    let mut next=start.to_owned();let mut depth=depth;
    let mut visited=std::collections::HashSet::new();
    for _ in 0..8 {
        if !visited.insert(next.clone()){return Err(Failure::text("Calendar server returned a looping discovery address"));}
        let (xml,base)=match dav_response(agent,"PROPFIND",&next,depth,PROPFIND_BODY,cfg,password) {
            Ok(response)=>response,
            Err(error)=>{
                // 仅跳过初始入口自身的错误，重定向或属性发现后的服务错误原样返回。
                let initial=visited.len()==1 && error.request_url.as_deref()==Some(start);
                let unavailable=match path_of(start) {
                    "/"=>matches!(error.status,Some(400|403|404|405)),
                    "/.well-known/caldav"=>matches!(error.status,Some(404|405)),
                    _=>false,
                };
                if initial && unavailable {return Ok(None);}
                return Err(error);
            }
        };
        visited.insert(base.clone());
        let calendars=calendar_collections(&xml,&base);
        if !calendars.is_empty(){return Ok(Some(calendars));}
        if let Some(home)=calendar_home(&xml,&base).filter(|url|!visited.contains(url)) {
            next=home;depth="1";
        } else if let Some(principal)=property_href(&xml,"current-user-principal",&base).filter(|url|!visited.contains(url)) {
            next=principal;depth="0";
        } else {return Ok(None);}
    }
    Err(Failure::text("Too many calendar discovery steps. Please reconnect"))
}

// ---------------------------------------------------------------- XML

/// 把 PROPFIND/REPORT 的 207 拆成 `(href, 是否日历集合, 是否目录)`。
fn responses(xml: &str) -> Vec<(String, bool, bool)> {
    let Ok(block) = regex::Regex::new(r"(?is)<(?:[a-z0-9]+:)?response\b[^>]*>(.*?)</(?:[a-z0-9]+:)?response>") else {
        return Vec::new();
    };
    let Ok(href_re) = regex::Regex::new(r"(?is)<(?:[a-z0-9]+:)?href\b[^>]*>(.*?)</(?:[a-z0-9]+:)?href>") else {
        return Vec::new();
    };
    let resource_type=regex::Regex::new(r"(?is)<(?:[a-z0-9]+:)?resourcetype\b[^>]*>(.*?)</(?:[a-z0-9]+:)?resourcetype>").unwrap();
    let calendar_type=regex::Regex::new(r"(?is)<(?:[a-z0-9]+:)?calendar(?:\s[^>]*)?/?>").unwrap();
    block
        .captures_iter(xml)
        .filter_map(|cap| {
            let inner = cap.get(1)?.as_str();
            let href = unescape_xml(href_re.captures(inner)?.get(1)?.as_str().trim());
            if href.is_empty() {
                return None;
            }
            let is_calendar = resource_type.captures_iter(inner).any(|kind|calendar_type.is_match(&kind[1]));
            let is_dir = href.ends_with('/');
            Some((href, is_calendar, is_dir))
        })
        .collect()
}

fn calendar_home(xml: &str, base: &str) -> Option<String> {
    property_href(xml,"calendar-home-set",base)
}

fn property_href(xml:&str,property:&str,base:&str)->Option<String>{
    let block = regex::Regex::new(&format!(r"(?is)<(?:[a-z0-9]+:)?{property}\b[^>]*>(.*?)</(?:[a-z0-9]+:)?{property}>")).ok()?;
    let href_re = regex::Regex::new(r"(?is)<(?:[a-z0-9]+:)?href\b[^>]*>(.*?)</(?:[a-z0-9]+:)?href>").ok()?;
    let inner = block.captures(xml)?.get(1)?.as_str();
    let href = href_re.captures(inner)?.get(1)?.as_str().trim();
    if href.is_empty() {
        return None;
    }
    url::Url::parse(base).ok()?.join(&unescape_xml(href)).ok().map(|url|url.to_string())
}

fn calendar_collections(xml:&str,base:&str)->Vec<String>{
    // 只取能放日程（VEVENT）的日历集合，跳过只放提醒事项的列表，最多 12 个。
    let mut calendars=event_calendars(xml,base);
    if calendars.is_empty(){if let Some(url)=pick_calendar(xml,base){calendars.push(url);}}
    calendars
}

#[cfg(test)]
fn discover(agent:&ureq::Agent,cfg:&Config,password:&str)->Result<String,Failure>{
    discover_calendars(agent,cfg,password)?.into_iter().next().ok_or_else(||Failure::text("没有找到日历"))
}

/// 只选择日历集合或已有的 UUID 兼容路径，不能把主目录/收件箱当成日历。
fn pick_calendar(xml: &str, base: &str) -> Option<String> {
    let want = path_of(base).trim_end_matches('/').to_string();
    let mut best: Option<(u8, String)> = None;
    for (href, is_calendar, _) in responses(xml) {
        let Some(url) = url::Url::parse(base).ok().and_then(|base|base.join(&href).ok()).map(|url|url.to_string()) else {continue;};
        let path = path_of(&url).trim_end_matches('/').to_string();
        if path.is_empty() || path == want && !is_calendar {
            continue;
        }
        if !is_calendar && (path.contains("principals") || path.contains("calendar-home")) {
            continue;
        }
        let score = if is_calendar { 3 } else if looks_like_uuid(path.rsplit('/').next().unwrap_or("")) { 2 } else { continue; };
        if best.as_ref().map_or(true, |(s, _)| score > *s) {
            best = Some((score, url));
        }
    }
    best.map(|(_, url)| url)
}

/// 账号下所有能放日程（VEVENT）的日历集合；只放提醒事项（VTODO）的跳过。最多 12 个。
fn event_calendars(xml: &str, base: &str) -> Vec<String> {
    let Ok(block) = regex::Regex::new(r"(?is)<(?:[a-z0-9]+:)?response\b[^>]*>(.*?)</(?:[a-z0-9]+:)?response>") else { return Vec::new(); };
    let set = regex::Regex::new(r"(?is)<(?:[a-z0-9]+:)?supported-calendar-component-set\b[^>]*>(.*?)</(?:[a-z0-9]+:)?supported-calendar-component-set>").unwrap();
    let vevent = regex::Regex::new(r#"(?is)name\s*=\s*["']VEVENT["']"#).unwrap();
    let Ok(base_url) = url::Url::parse(base) else { return Vec::new(); };
    let mut out = Vec::new();
    for cap in block.captures_iter(xml) {
        let inner = &cap[1];
        // 复用 `responses` 的判定：单独解析这一段。
        let Some((href, is_calendar, _)) = responses(&cap[0]).into_iter().next() else { continue; };
        if !is_calendar { continue; }
        if let Some(components) = set.captures(inner) {
            if !vevent.is_match(&components[1]) { continue; }
        }
        let Ok(url) = base_url.join(&href) else { continue; };
        let url = url.to_string();
        if !out.contains(&url) { out.push(url); }
        if out.len() >= 12 { break; }
    }
    out
}

/// 从 .ics 文件（苹果「日历」导出、订阅链接下载等）读出时间范围内的日程，重复日程会展开。
pub fn events_from_ics(ics: &str, lo: chrono::DateTime<chrono::Utc>, hi: chrono::DateTime<chrono::Utc>) -> Result<Vec<crate::mail::Meeting>, String> {
    if !ics.contains("BEGIN:VCALENDAR") && !ics.contains("BEGIN:VEVENT") { return Err("Not a valid calendar file (.ics)".into()); }
    let escaped = ics.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let xml = format!("<d:response><c:calendar-data>{escaped}</c:calendar-data></d:response>");
    events_in_range(&xml, lo, hi).map_err(|e| e.message)
}

fn calendar_data(xml: &str) -> Vec<String> {
    let Ok(re) = regex::Regex::new(r"(?is)<(?:[a-z0-9]+:)?calendar-data\b[^>]*>(.*?)</(?:[a-z0-9]+:)?calendar-data>") else {
        return Vec::new();
    };
    re.captures_iter(xml)
        .filter_map(|cap| {
            let raw = cap.get(1)?.as_str();
            Some(match raw.trim().strip_prefix("<![CDATA[").and_then(|s|s.strip_suffix("]]>")) {
                Some(text)=>text.to_string(),
                None=>unescape_xml(raw),
            })
        })
        .collect()
}

/// 把 REPORT 里的所有 `calendar-data` 喂给现有 .ics 解析器，按 `UID + DTSTART`
/// 去重（同一个约会从多个日历返回也只留一条）。
pub fn events_from_report(xml: &str) -> Vec<crate::mail::Meeting> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for data in calendar_data(xml) {
        for m in crate::mail::meetings_of(&data) {
            if seen.insert(dedupe_key(&m)) {
                out.push(m);
            }
        }
    }
    out
}

pub fn dedupe_key(m: &crate::mail::Meeting) -> String {
    if m.uid.is_empty() {
        format!("t{}|{}", m.start,m.title.trim())
    } else {
        format!("{}|{}", m.uid, m.start)
    }
}

fn unescape_xml(v: &str) -> String {
    let re=regex::Regex::new(r"&(#x[0-9a-fA-F]+|#[0-9]+|lt|gt|quot|apos|amp);").unwrap();
    re.replace_all(v,|c:&regex::Captures|{
        let token=&c[1];
        let ch=match token {
            "lt"=>Some('<'),"gt"=>Some('>'),"quot"=>Some('"'),"apos"=>Some('\''),"amp"=>Some('&'),
            _=>token.strip_prefix("#x").and_then(|n|u32::from_str_radix(n,16).ok()).or_else(||token.strip_prefix('#').and_then(|n|n.parse().ok())).and_then(char::from_u32),
        };
        ch.map(|v|v.to_string()).unwrap_or_else(||c[0].to_string())
    }).trim_matches('\u{feff}').to_string()
}

fn report_status_error(xml:&str)->Option<u16>{
    regex::Regex::new(r"(?i)<(?:[a-z0-9]+:)?status[^>]*>\s*HTTP/\S+\s+(\d{3})").unwrap().captures_iter(xml)
        .filter_map(|c|c[1].parse::<u16>().ok()).find(|code|*code>=400)
}

/// 服务器未展开重复日程时，在查询窗口内展开，并用单次修改或取消覆盖原实例。
fn events_in_range(xml:&str,lo:chrono::DateTime<chrono::Utc>,hi:chrono::DateTime<chrono::Utc>)->Result<Vec<crate::mail::Meeting>,Failure>{
    let mut blocks=Vec::new();
    for data in calendar_data(xml){
        let unfolded=data.replace("\r\n","\n").replace("\n ","").replace("\n\t","");
        if unfolded.lines().any(|line|line.eq_ignore_ascii_case("METHOD:CANCEL")){continue;}
        for part in unfolded.split("BEGIN:VEVENT").skip(1){
            if let Some((body,_))=part.split_once("END:VEVENT") {blocks.push(body.to_string());}
        }
    }
    let field=|body:&str,name:&str|body.lines().find(|line|line.split([';',':']).next()==Some(name)).map(str::to_string);
    let mut overrides=std::collections::HashSet::new();
    for body in &blocks {
        if let Some(id)=field(body,"RECURRENCE-ID"){
            if id.contains("RANGE="){return Err(Failure::text("Recurring events changed in bulk from a given occurrence aren’t supported yet"));}
            let synthetic=format!("BEGIN:VEVENT\n{}\nEND:VEVENT",id.replacen("RECURRENCE-ID","DTSTART",1));
            let Some(at)=crate::mail::meetings_of(&synthetic).first().map(|m|m.start) else{return Err(Failure::text("Can’t parse the modified time of a recurring event"));};
            overrides.insert((field(body,"UID").unwrap_or_default(),at));
        }
    }
    let mut events=Vec::new();
    for body in blocks {
        let raw=format!("BEGIN:VEVENT{body}END:VEVENT");
        let Some(base)=crate::mail::meetings_of(&raw).into_iter().next() else{
            if field(&body,"STATUS").is_some_and(|s|s.split_once(':').is_some_and(|(_,v)|v.trim().eq_ignore_ascii_case("CANCELLED"))){continue;}
            return Err(Failure::text("Server returned events, but their time or time zone couldn’t be parsed. Sync not completed"));
        };
        if field(&body,"RECURRENCE-ID").is_some() || field(&body,"RRULE").is_none() {
            if field(&body,"RECURRENCE-ID").is_some() || !overrides.contains(&(field(&body,"UID").unwrap_or_default(),base.start)){events.push(base);}
            continue;
        }
        let mut rules=Vec::new();let mut nested=0;
        for line in body.lines(){
            if line.starts_with("BEGIN:"){nested+=1;continue;}
            if line.starts_with("END:"){nested-=1;continue;}
            if nested==0 && matches!(line.split([';',':']).next(),Some("DTSTART"|"RRULE"|"RDATE"|"EXDATE")){rules.push(line);}
        }
        let set:rrule::RRuleSet=rules.join("\n").parse().map_err(|_|Failure::text("Can’t parse the server’s recurrence rules"))?;
        let duration=base.end.map_or(0,|end|end-base.start);
        let result=set.after((lo-chrono::Duration::seconds(duration.max(1))).with_timezone(&rrule::Tz::UTC)).before(hi.with_timezone(&rrule::Tz::UTC)).all(2000);
        if result.limited{return Err(Failure::text("Too many recurring events to fully sync the current time range"));}
        let uid=field(&body,"UID").unwrap_or_default();
        for date in result.dates {
            let start=date.timestamp();if overrides.contains(&(uid.clone(),start)){continue;}
            let mut event=base.clone();event.start=start;event.end=base.end.map(|end|start+end-base.start);
            event.at=date.with_timezone(&chrono::Local).format(if event.all_day {"%m-%d All day"}else{"%m-%d %H:%M"}).to_string();events.push(event);
        }
    }
    events.retain(|m|m.start<hi.timestamp()&&m.end.unwrap_or(m.start+1)>lo.timestamp());
    events.sort_by_key(|m|m.start);let mut seen=std::collections::HashSet::new();events.retain(|m|seen.insert(dedupe_key(m)));Ok(events)
}

// ---------------------------------------------------------------- URL

pub fn ensure_slash(base: &str) -> String {
    if base.ends_with('/') { base.to_string() } else { format!("{base}/") }
}

pub fn join_url(base: &str, href: &str) -> String {
    url::Url::parse(&ensure_slash(base)).and_then(|base|base.join(href.trim()))
        .map(|url|url.to_string()).unwrap_or_default()
}

pub fn path_of(url: &str) -> &str {
    let Some(scheme_end) = url.find("://") else { return url };
    let rest = &url[scheme_end + 3..];
    match rest.find('/') {
        Some(i) => &rest[i..],
        None => "/",
    }
}

fn looks_like_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

// ---------------------------------------------------------------- 后台服务

#[derive(Clone)]
pub enum Cmd {
    /// 配置变化（空列表 = 关闭）。
    Configure(Vec<Config>, bool),
    /// 立即同步一次。
    SyncNow,
    ViewMonth(NaiveDate),
    /// 用临时密码试连，成功后界面才落盘。
    Test(Config, String),
}

pub enum Event {
    /// 试连结果；`count` 是未来 24 小时的事件数，`config` 带回发现到的日历地址。
    Test { ok: bool, message: String, count: usize, config: Option<Config> },
    /// 一次成功同步。
    Synced { config: Config, events: Vec<crate::mail::Meeting>, at: String },
    MonthLoaded { config: Config, month: NaiveDate, result: Result<Vec<crate::mail::Meeting>, String> },
    /// 一次失败（界面只在连续失败时显示红字）。
    Failed { config: Config, message: String },
}

pub struct Service {
    cmd: Sender<Cmd>,
    ev: Receiver<Event>,
}

impl Service {
    pub fn start() -> Self {
        let (cmd_tx, cmd_rx) = channel();
        let (ev_tx, ev_rx) = channel();
        let _ = std::thread::Builder::new()
            .name("daozai-calendar".into())
            .spawn(move || run(cmd_rx, ev_tx));
        Self { cmd: cmd_tx, ev: ev_rx }
    }

    pub fn configure(&self, cfg: Vec<Config>, auto_sync: bool) {
        let _ = self.cmd.send(Cmd::Configure(cfg, auto_sync));
    }

    pub fn sync_now(&self) {
        let _ = self.cmd.send(Cmd::SyncNow);
    }

    pub fn view_month(&self, month: NaiveDate) {
        let _ = self.cmd.send(Cmd::ViewMonth(month_start(month)));
    }

    pub fn test(&self, cfg: Config, password: String) {
        let _ = self.cmd.send(Cmd::Test(cfg, password));
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.ev.try_recv().ok()
    }
}

fn run(cmd_rx: Receiver<Cmd>, ev: Sender<Event>) {
    run_with(cmd_rx, ev, Duration::from_secs(SYNC_SECS), sync_once);
}

fn run_with(cmd_rx: Receiver<Cmd>, ev: Sender<Event>, interval: Duration, mut sync: impl FnMut(&Config, Option<NaiveDate>, &Sender<Event>)) {
    let mut cfg: Vec<Config> = Vec::new();
    let mut auto_sync = true;
    let mut next: Option<Instant> = None;
    let mut month = None;
    loop {
        let message = match next {
            Some(at) => cmd_rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => cmd_rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let mut dirty = false;
        match message {
            Ok(Cmd::Configure(c, enabled)) => {
                dirty = !c.is_empty() && (c != cfg || enabled && !auto_sync);
                cfg = c;
                auto_sync = enabled;
                if !auto_sync || cfg.is_empty() { next = None; }
                if cfg.is_empty() { month = None; }
            }
            Ok(Cmd::SyncNow) => dirty = true,
            Ok(Cmd::ViewMonth(date)) => { month = Some(month_start(date)); dirty = true; }
            Ok(Cmd::Test(c, password)) => {
                let ev = ev.clone();
                let _ = std::thread::Builder::new()
                    .name("daozai-calendar-test".into())
                    .spawn(move || {
                        let result = fetch(&c, &password);
                        let (ok, message, count, config) = match result {
                            Ok(f) => {
                                let mut saved = c.clone();
                                saved.href = Some(f.href);
                                (true, "Connected".to_string(), f.events.len(), Some(saved))
                            }
                            Err(e) => (false, e.message, 0, None),
                        };
                        let _ = ev.send(Event::Test { ok, message, count, config });
                    });
            }
            Err(RecvTimeoutError::Timeout) => dirty = true,
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if !dirty {
            continue;
        }
        if cfg.is_empty() { continue; }
        for c in &cfg { sync(c, month, &ev); }
        next = auto_sync.then(|| Instant::now() + interval);
    }
}

fn sync_once(
    cfg: &Config,
    month: Option<NaiveDate>,
    ev: &Sender<Event>,
) {
    let password = match secrets::load(cfg) {
        Ok(p) => p,
        Err(e) => {
            // 没配置凭据：静默，只记日志。
            eprintln!("[calendar] credentials unavailable: {e}");
            let _ = ev.send(Event::Failed { config:cfg.clone(),message: e.clone() });
            if let Some(month) = month { let _ = ev.send(Event::MonthLoaded { config: cfg.clone(), month, result: Err(e) }); }
            return;
        }
    };
    match fetch(cfg, &password) {
        Ok(f) => {
            let _ = ev.send(Event::Synced { config:cfg.clone(),events: f.events, at: now_label() });
        }
        Err(e) => {
            eprintln!("[calendar] sync failed: {}", e.message);
            let _ = ev.send(Event::Failed { config:cfg.clone(),message: e.message });
        }
    }
    if let Some(month) = month {
        let result = fetch_month(cfg, &password, month).map(|f| f.events).map_err(|e| e.message);
        let _ = ev.send(Event::MonthLoaded { config: cfg.clone(), month, result });
    }
}

pub fn now_label() -> String {
    chrono::Local::now().format("%m-%d %H:%M").to_string()
}

/// 把 HTTP 状态码翻成人话（设置页和录制演示共用）。
pub fn human_error(provider: Provider, status: Option<u16>) -> String {
    match status {
        Some(code) => Failure::status(provider, code).message,
        None => Failure::transport("").message,
    }
}

#[cfg(test)]
mod tests {
    fn serve_calendar_discovery(listener:&std::net::TcpListener){
        use std::io::Write;
        let (mut socket,_)=listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        assert!(read_discovery_request(&mut socket).starts_with("PROPFIND "));
        let xml="<d:multistatus><d:response><d:href>/calendar/</d:href><d:propstat><d:prop><d:resourcetype><c:calendar/></d:resourcetype></d:prop></d:propstat></d:response></d:multistatus>";
        write!(socket,"HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{xml}",xml.len()).unwrap();
    }

    #[test]
    fn saved_empty_calendar_does_not_hide_other_calendars(){
        use std::{io::Write,net::TcpListener};
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
        let base=format!("http://{}/",listener.local_addr().unwrap());
        let (stop,done)=std::sync::mpsc::channel();
        let worker=std::thread::spawn(move||{
            while done.try_recv().is_err(){
                let Ok((mut socket,_))=listener.accept() else{std::thread::sleep(Duration::from_millis(5));continue;};
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let request=read_discovery_request(&mut socket);
                let body=if request.starts_with("PROPFIND ") {
                    "<d:multistatus><d:response><d:href>/empty/</d:href><d:propstat><d:prop><d:resourcetype><c:calendar/></d:resourcetype></d:prop></d:propstat></d:response><d:response><d:href>/work/</d:href><d:propstat><d:prop><d:resourcetype><c:calendar/></d:resourcetype></d:prop></d:propstat></d:response></d:multistatus>"
                }else if request.starts_with("REPORT /work/ ") {
                    "<d:multistatus><d:response><d:href>/work/a.ics</d:href><c:calendar-data>BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:work-event\nDTSTART:20261104T020000Z\nSUMMARY:工作日程\nEND:VEVENT\nEND:VCALENDAR</c:calendar-data></d:response></d:multistatus>"
                }else{"<d:multistatus/>"};
                write!(socket,"HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let cfg=Config{provider:Provider::DingTalk,username:"test".into(),href:Some(format!("{base}empty/")),server:Some(base)};
        let result=fetch_month(&cfg,"test",NaiveDate::from_ymd_opt(2026,11,1).unwrap());
        stop.send(()).unwrap();worker.join().unwrap();
        assert_eq!(result.unwrap().events.len(),1,"已保存的空日历不能遮蔽其他日历");
    }

    #[test]
    fn malformed_events_are_not_reported_as_an_empty_calendar(){
        let lo="2026-11-01T00:00:00Z".parse().unwrap();let hi="2026-12-01T00:00:00Z".parse().unwrap();
        let xml="<calendar-data>BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:test\nDTSTART;TZID=Unsupported/Zone:20261104T100000\nEND:VEVENT\nEND:VCALENDAR</calendar-data>";
        assert!(events_in_range(xml,lo,hi).is_err());
        let cancelled=xml.replace("UID:test","UID:test\nSTATUS:CANCELLED");
        assert!(events_in_range(&cancelled,lo,hi).unwrap().is_empty());
    }

    #[test]
    fn wecom_uses_exchange_endpoint() {
        assert_eq!(Provider::WeCom.server(), "https://wecom.work");
    }
    #[test]
    fn calendar_resource_refs_accept_object_slashes_and_skip_collection_metadata() {
        let xml=r#"<d:multistatus><d:response><d:href>/calendar/</d:href><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:response><d:response><d:href>/calendar/event/?id=1&amp;version=2</d:href></d:response></d:multistatus>"#;
        assert_eq!(report_resource_refs(xml,"https://caldav.wecom.work/calendar/").unwrap(),"<d:href>/calendar/event/?id=1&amp;version=2</d:href>");
        assert!(report_resource_refs("<response><href>https://other.example/item.ics</href></response>","https://caldav.wecom.work/calendar/").is_err());
    }
    fn read_discovery_request(stream:&mut std::net::TcpStream)->String {
        use std::io::Read;
        // Windows 上从非阻塞监听接受的连接会继承非阻塞模式，请求未到时读取会直接报 WouldBlock。
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut bytes=Vec::new();
        loop {
            let mut buffer=[0;4096];let n=stream.read(&mut buffer).unwrap();
            if n==0 {break;} bytes.extend_from_slice(&buffer[..n]);
            let text=String::from_utf8_lossy(&bytes);
            if let Some(end)=text.find("\r\n\r\n") {
                let length=text[..end].lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0);
                if bytes.len()>=end+4+length {break;}
            }
        }
        String::from_utf8(bytes).unwrap()
    }
    #[test]
    fn discovery_root_errors_follow_well_known_principal_and_home() {
        use std::io::Write;
        use std::net::TcpListener;
        for (provider, code) in [(Provider::DingTalk,405),(Provider::WeCom,403),(Provider::ICloud,400),(Provider::DingTalk,400),(Provider::Custom,400)] {
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base=format!("http://{}/",listener.local_addr().unwrap());
            let (stop_tx,stop_rx)=std::sync::mpsc::channel();
            let worker=std::thread::spawn(move || {
                let mut paths=Vec::new();
                while stop_rx.try_recv().is_err() {
                    let Ok((mut stream,_))=listener.accept() else {std::thread::sleep(Duration::from_millis(5));continue;};
                    let request=read_discovery_request(&mut stream);
                    let path=request.split_whitespace().nth(1).unwrap().to_owned();paths.push(path.clone());
                    let (status,extra,body)=match path.as_str() {
                        "/"=>(code,"",""),
                        "/.well-known/caldav"=>(302,"Location: /dav/principals/\r\n",""),
                        "/dav/principals/"=>(207,"",r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/dav/principals/</d:href><d:propstat><d:prop><d:current-user-principal><d:href>account/</d:href></d:current-user-principal></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#),
                        "/dav/principals/account/"=>(207,"",r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/dav/principals/account/</d:href><d:propstat><d:prop><c:calendar-home-set><d:href>../../calendars/account/</d:href></c:calendar-home-set></d:prop></d:propstat></d:response></d:multistatus>"#),
                        "/dav/calendars/account/"=>(207,"",r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/dav/calendars/account/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response><d:response><d:href>primary/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop></d:propstat></d:response></d:multistatus>"#),
                        _=>(404,"","")
                    };
                    write!(stream,"HTTP/1.1 {status} Test\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
                paths
            });
            let cfg=Config{provider,username:"synthetic".into(),href:None,server:Some(base.clone())};
            let result=discover(&agent(),&cfg,"synthetic-password");
            stop_tx.send(()).unwrap();let paths=worker.join().unwrap();
            assert_eq!(result.unwrap(),format!("{base}dav/calendars/account/primary/"));
            assert_eq!(paths,vec!["/","/.well-known/caldav","/dav/principals/","/dav/principals/account/","/dav/calendars/account/"]);
        }
    }

    #[test]
    fn discovery_errors_at_resolved_service_preserve_status_and_stop() {
        use std::{io::Write,net::TcpListener};
        for code in [400,403,404,405] {
            for well_known in [false,true] {
                for redirect in [false,true] {
                    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
                    listener.set_nonblocking(true).unwrap();
                    let base=format!("http://{}/",listener.local_addr().unwrap());
                    let (stop,done)=std::sync::mpsc::channel();
                    let worker=std::thread::spawn(move || {
                        let mut paths=Vec::new();
                        while done.try_recv().is_err() {
                            let Ok((mut stream,_))=listener.accept() else {std::thread::sleep(Duration::from_millis(5));continue;};
                            let request=read_discovery_request(&mut stream);
                            let path=request.split_whitespace().nth(1).unwrap().to_owned();
                            paths.push(path.clone());
                            let principal=r#"<d:multistatus><d:response><d:href>/</d:href><d:propstat><d:prop><d:current-user-principal><d:href>/account/</d:href></d:current-user-principal></d:prop></d:propstat></d:response></d:multistatus>"#;
                            let (status,extra,body)=match path.as_str() {
                                "/" if well_known=>(405,"",""),
                                "/"|"/.well-known/caldav" if redirect=>(302,"Location: /account/\r\n",""),
                                "/"|"/.well-known/caldav"=>(207,"",principal),
                                "/account/"=>(code,"",""),
                                _=>(404,"",""),
                            };
                            write!(stream,"HTTP/1.1 {status} Test\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                        }
                        paths
                    });
                    let cfg=Config{provider:Provider::ICloud,username:"synthetic".into(),href:None,server:Some(base)};
                    let result=fetch_month(&cfg,"synthetic-password",NaiveDate::from_ymd_opt(2026,11,1).unwrap());
                    stop.send(()).unwrap();
                    let paths=worker.join().unwrap();
                    let error=result.err().unwrap();
                    assert_eq!(error.status,Some(code),"well_known={well_known} redirect={redirect} paths={paths:?}");
                    let expected=if well_known {vec!["/","/.well-known/caldav","/account/"]} else {vec!["/","/account/"]};
                    assert_eq!(paths,expected,"实际日历服务失败后应保留错误");
                    assert!(error.message.contains("查找日历"),"{}",error.message);
                }
            }
        }
    }

    #[test]
    fn explicit_calendar_path_errors_do_not_probe_other_entries() {
        use std::{io::Write,net::TcpListener};
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let base=format!("http://{}/dav/",listener.local_addr().unwrap());
        let worker=std::thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap();
            assert!(read_discovery_request(&mut stream).starts_with("PROPFIND /dav/ "));
            write!(stream,"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            drop(stream);listener
        });
        let cfg=Config{provider:Provider::Custom,username:"synthetic".into(),href:None,server:Some(base)};
        assert_eq!(discover(&agent(),&cfg,"synthetic-password").unwrap_err().status,Some(400));
        let listener=worker.join().unwrap();listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err());
    }

    #[test]
    fn report_errors_identify_the_failed_stage() {
        use std::{io::Write,net::TcpListener};
        for code in [400,401,403,404,405,429,500] {
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();
            let base=format!("http://{}/",listener.local_addr().unwrap());
            let worker=std::thread::spawn(move || {
                serve_calendar_discovery(&listener);
                let (mut stream,_)=listener.accept().unwrap();
                assert!(read_discovery_request(&mut stream).starts_with("REPORT /calendar/ "));
                write!(stream,"HTTP/1.1 {code} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            });
            let cfg=Config{provider:Provider::Custom,username:"synthetic".into(),href:None,server:Some(base)};
            let error=fetch_month(&cfg,"synthetic-password",NaiveDate::from_ymd_opt(2026,11,1).unwrap()).err().unwrap();
            worker.join().unwrap();
            assert_eq!(error.status,Some(code));
            assert!(error.message.contains(if code==401 {"验证账号失败"} else {"读取日程失败"}),"{}",error.message);
            assert!(error.message.contains(&code.to_string()),"{}",error.message);
        }
    }

    #[test]
    fn discovery_only_selects_calendar_collections_including_self() {
        let xml=r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/dav/account/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><c:calendar-home-set><d:href>/dav/home/</d:href></c:calendar-home-set></d:prop></d:propstat></d:response></d:multistatus>"#;
        assert_eq!(pick_calendar(xml,"https://example.com/"),None,"日历主目录属性不能被当成日历集合类型");
        let calendar=xml.replace("<d:collection/>","<d:collection/><c:calendar/>");
        assert_eq!(pick_calendar(&calendar,"https://example.com/dav/account/"),Some("https://example.com/dav/account/".into()));
    }

    #[test]
    fn discovery_authentication_failure_does_not_probe_other_paths() {
        use std::io::Write;use std::net::TcpListener;
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let base=format!("http://{}/",listener.local_addr().unwrap());
        let worker=std::thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap();let _=read_discovery_request(&mut stream);
            write!(stream,"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            drop(stream);listener
        });
        let cfg=Config{provider:Provider::DingTalk,username:"synthetic".into(),href:None,server:Some(base)};
        let client=ureq::AgentBuilder::new().timeout(Duration::from_millis(200)).build();
        assert_eq!(discover(&client,&cfg,"wrong").unwrap_err().status,Some(401));
        let listener=worker.join().unwrap();listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err());
    }

    #[test]
    fn discovery_principal_cannot_send_credentials_to_another_origin() {
        use std::io::Write;use std::net::TcpListener;
        let foreign=TcpListener::bind("127.0.0.1:0").unwrap();
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let base=format!("http://{}/",listener.local_addr().unwrap());
        let foreign_url=format!("http://{}/account/",foreign.local_addr().unwrap());
        let worker=std::thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap();let _=read_discovery_request(&mut stream);
            let body=format!("<d:current-user-principal xmlns:d=\"DAV:\"><d:href>{foreign_url}</d:href></d:current-user-principal>");
            write!(stream,"HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        });
        let cfg=Config{provider:Provider::WeCom,username:"synthetic".into(),href:None,server:Some(base)};
        assert!(discover(&agent(),&cfg,"synthetic-password").unwrap_err().message.contains("不可信"));
        worker.join().unwrap();foreign.set_nonblocking(true).unwrap();assert!(foreign.accept().is_err());
    }

    #[test]
    fn quality_dav_never_sends_credentials_to_an_unrelated_origin() {
        use std::net::TcpListener;
        let server=TcpListener::bind("127.0.0.1:0").unwrap();
        let url=format!("http://{}/calendar/",server.local_addr().unwrap());
        let cfg=Config{provider:Provider::Feishu,username:"synthetic".into(),href:Some(url.clone()),server:None};
        let client=ureq::AgentBuilder::new().timeout(Duration::from_millis(100)).build();
        assert!(dav(&client,"REPORT",&url,"1","",&cfg,"synthetic-secret").is_err());
        server.set_nonblocking(true).unwrap();
        assert!(server.accept().is_err(),"不可信地址不能收到携带凭据的请求");
    }

    #[test]
    fn quality_dav_redirects_keep_the_method_and_reject_foreign_hosts() {
        use std::io::{Read,Write};use std::net::TcpListener;
        let foreign=TcpListener::bind("127.0.0.1:0").unwrap();
        let server=TcpListener::bind("127.0.0.1:0").unwrap();
        let base=format!("http://{}/",server.local_addr().unwrap());
        let foreign_url=format!("http://{}/",foreign.local_addr().unwrap());
        let worker=std::thread::spawn(move||{
            for (i,(status,location)) in [("307 Temporary Redirect","/next"),("207 Multi-Status",""),("302 Found",foreign_url.as_str())].into_iter().enumerate(){
                let (mut stream,_)=server.accept().unwrap();stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request=String::new();
                while !request.contains("\r\n\r\n"){let mut buf=[0;1024];let n=stream.read(&mut buf).unwrap();assert!(n>0);request.push_str(&String::from_utf8_lossy(&buf[..n]));}
                assert!(request.starts_with(if i==1{"REPORT /next "}else{"REPORT / "}));
                assert!(request.to_ascii_lowercase().contains("authorization: basic "));
                write!(stream,"HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let cfg=Config{provider:Provider::Feishu,username:"synthetic".into(),href:None,server:Some(base.clone())};
        assert!(dav(&agent(),"REPORT",&base,"1","",&cfg,"secret").is_ok());
        assert!(dav(&agent(),"REPORT",&base,"1","",&cfg,"secret").is_err());
        worker.join().unwrap();foreign.set_nonblocking(true).unwrap();assert!(foreign.accept().is_err());
    }

    #[test]
    fn quality_cdata_and_calendar_urls_preserve_literal_content() {
        assert_eq!(calendar_data("<c:calendar-data><![CDATA[SUMMARY:A &amp; B]]></c:calendar-data>"),vec!["SUMMARY:A &amp; B"]);
        assert_eq!(calendar_home("<c:calendar-home-set><d:href>/calendar/?a=1&amp;b=2</d:href></c:calendar-home-set>","https://a.com/"),Some("https://a.com/calendar/?a=1&b=2".into()));
        assert_eq!(join_url("https://a.com/dav/user/","../team/"),"https://a.com/dav/team/");
    }

    #[test]
    fn quality_different_meetings_without_uid_are_not_merged() {
        let a=crate::mail::Meeting{ all_day:false,title:"设计评审".into(),start:100,end:None,at:String::new(),uid:String::new()};
        let b=crate::mail::Meeting{ all_day:false,title:"客户会议".into(),..a.clone()};
        assert_ne!(dedupe_key(&a),dedupe_key(&b));
    }
    #[test]
    fn recurring_events_respect_edits_cancellations_and_exdates(){
        let data="BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:daily\nSUMMARY:原标题\nDTSTART;TZID=Asia/Shanghai:20261103T112000\nDTEND;TZID=Asia/Shanghai:20261103T115000\nRRULE:FREQ=DAILY;COUNT=4\nEXDATE;TZID=Asia/Shanghai:20261106T112000\nEND:VEVENT\nBEGIN:VEVENT\nUID:daily\nSUMMARY:修改标题\nRECURRENCE-ID;TZID=Asia/Shanghai:20261103T112000\nDTSTART;TZID=Asia/Shanghai:20261103T112000\nDTEND;TZID=Asia/Shanghai:20261103T115000\nEND:VEVENT\nBEGIN:VEVENT\nUID:daily\nRECURRENCE-ID;TZID=Asia/Shanghai:20261104T112000\nDTSTART;TZID=Asia/Shanghai:20261104T112000\nSTATUS:CANCELLED\nEND:VEVENT\nEND:VCALENDAR";
        let xml=format!("<c:calendar-data>{}</c:calendar-data>",data.replace('\n',"&#xD;&#xA;"));
        let lo="2026-11-01T00:00:00Z".parse().unwrap();let hi="2026-12-01T00:00:00Z".parse().unwrap();
        let events=events_in_range(&xml,lo,hi).unwrap();
        assert_eq!(events.len(),2);assert_eq!(events[0].title,"修改标题");assert_eq!(events[1].title,"原标题");
        assert_eq!(events[1].start-events[0].start,2*86400);assert_eq!(events[0].end.unwrap()-events[0].start,1800);
        let cancelled=format!("<c:calendar-data>BEGIN:VCALENDAR\nMETHOD:CANCEL\n{data}</c:calendar-data>");
        assert!(events_in_range(&cancelled,lo,hi).unwrap().is_empty());
    }

    #[test]
    fn recurring_all_day_and_overnight_overlap_query_start() {
        let lo=NaiveDate::from_ymd_opt(2026,11,4).unwrap().and_hms_opt(0,0,0).unwrap().and_local_timezone(chrono::Local).earliest().unwrap().with_timezone(&chrono::Utc);
        let xml="<c:calendar-data>BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:all-day\nDTSTART;VALUE=DATE:20261103\nDTEND;VALUE=DATE:20261104\nRRULE:FREQ=DAILY;COUNT=3\nEND:VEVENT\nBEGIN:VEVENT\nUID:overnight\nDTSTART:20261103T233000\nDTEND:20261104T003000\nRRULE:FREQ=DAILY;COUNT=2\nEND:VEVENT\nEND:VCALENDAR</c:calendar-data>";
        let events=events_in_range(xml,lo,lo+chrono::Duration::days(1)).unwrap();
        assert_eq!(events.iter().filter(|m|m.all_day).count(),1);
        assert_eq!(events.iter().filter(|m|m.uid=="overnight").count(),2);
    }

    #[test]
    fn recurring_weekly_time_preserves_dst(){
        let xml="<c:calendar-data>BEGIN:VEVENT\nUID:weekly\nDTSTART;TZID=America/New_York:20261025T090000\nRRULE:FREQ=WEEKLY;COUNT=2\nEND:VEVENT</c:calendar-data>";
        let events=events_in_range(xml,"2026-10-01T00:00:00Z".parse().unwrap(),"2026-12-01T00:00:00Z".parse().unwrap()).unwrap();
        assert_eq!(events.len(),2);assert_eq!(events[1].start-events[0].start,7*86400+3600);
    }

    #[test]
    fn feishu_hex_entities_preserve_event_lines(){
        let xml="<C:calendar-data>BEGIN:VCALENDAR&#xD;&#xA;BEGIN:VEVENT&#xD;&#xA;UID:test&#xD;&#xA;SUMMARY:测试&#xD;&#xA;DTSTART:20261103T032000Z&#xD;&#xA;END:VEVENT&#xD;&#xA;END:VCALENDAR</C:calendar-data>";
        assert_eq!(events_from_report(xml).len(),1);
        assert_eq!(unescape_xml("&amp;#xA;"),"&#xA;");
    }

    #[test]
    #[ignore="显式执行已连接日历的只读实测"]
    fn live_calendar_read_only_probe(){
        let configs=load_configs();assert!(!configs.is_empty(),"尚未保存日历连接");
        for cfg in configs {
        let password=secrets::load(&cfg).expect("日历凭据不可用");
        let day=fetch(&cfg,&password).expect("提醒窗口同步失败");
        let month=fetch_month(&cfg,&password,chrono::Local::now().date_naive()).expect("月份同步失败");
        let today=chrono::Local::now().date_naive();
        println!("provider={} day_events={} month_events={} today_events={}",cfg.provider.key(),day.events.len(),month.events.len(),month.events.iter().filter(|m|chrono::DateTime::from_timestamp(m.start,0).unwrap().with_timezone(&chrono::Local).date_naive()==today).count());
        }
    }

    #[test]
    #[ignore="显式验证已连接账号重新发现日历，不输出账号或日程内容"]
    fn live_calendar_discovery_read_only_probe(){
        let mut cfg=load_config().expect("尚未保存日历连接");
        let password=secrets::load(&cfg).expect("日历凭据不可用");
        cfg.href=None;
        let fetched=fetch(&cfg,&password).expect("重新发现及查询日历失败");
        assert!(!fetched.href.is_empty());
    }

    #[test]
    #[ignore="需要本轮创建的飞书个人测试日程"]
    fn live_calendar_test_samples() {
        let cfg=load_config().unwrap();let password=secrets::load(&cfg).unwrap();
        let month=fetch_month(&cfg,&password,NaiveDate::from_ymd_opt(2026,11,3).unwrap()).unwrap();
        let samples=month.events.iter().filter(|m|m.title.starts_with("【岛仔测试】")).collect::<Vec<_>>();
        let repeated=samples.iter().filter(|m|m.title.contains("重复与取消验证")).collect::<Vec<_>>();
        assert_eq!(repeated.len(),2);
        assert_eq!(repeated.iter().map(|m|chrono::DateTime::from_timestamp(m.start,0).unwrap().with_timezone(&chrono::Local).day()).collect::<Vec<_>>(),vec![3,5]);
        assert!(samples.iter().any(|m|m.all_day));
        let overnight=samples.iter().find(|m|m.title.contains("跨午夜")).unwrap();
        assert!(overnight.end.unwrap()>overnight.start);
        println!("personal_test_events={} repeated_dates=3,5 all_day=true overnight=true",samples.len());
    }

    use super::*;
    use std::io::Read;

    #[test]
    fn query_missing_data_retries_multiget_and_rejects_nested_errors() {
        use std::{io::Write,net::TcpListener};
        for succeeds in [true,false] {
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();
            let base=format!("http://{}/",listener.local_addr().unwrap());
            let server=std::thread::spawn(move||{
            serve_calendar_discovery(&listener);
                for index in 0..2 {
                    let (mut socket,_)=listener.accept().unwrap();
                    socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut buf=Vec::new();let mut chunk=[0u8;4096];
                    loop {
                        let n=socket.read(&mut chunk).unwrap();assert!(n>0);buf.extend_from_slice(&chunk[..n]);
                        if let Some(pos)=buf.windows(4).position(|s|s==b"\r\n\r\n") {
                            let head=String::from_utf8_lossy(&buf[..pos]);
                            let len=head.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                            if buf.len()>=pos+4+len {break;}
                        }
                    }
                    let request=String::from_utf8(buf).unwrap();
                    assert!(request.contains(if index==0 {"calendar-query"} else {"calendar-multiget"}));
                    assert!(request.contains("<d:getetag/>"),"钉钉要求查询与批量读取同时请求 ETag，否则返回 400");
                    if index==1 {assert!(request.contains("<d:href>/calendar/item.ics</d:href>"));}
                    let data=if index==1 && succeeds {"BEGIN:VCALENDAR&#xD;&#xA;BEGIN:VEVENT&#xD;&#xA;UID:test&#xD;&#xA;SUMMARY:测试&#xD;&#xA;DTSTART:20261103T032000Z&#xD;&#xA;RRULE:FREQ=DAILY;COUNT=2&#xD;&#xA;END:VEVENT&#xD;&#xA;END:VCALENDAR"} else {""};
                    let status=if data.is_empty(){404}else{200};
                    let xml=format!("<d:multistatus><d:response><d:href>/calendar/item.ics</d:href><d:propstat><d:prop><c:calendar-data>{data}</c:calendar-data></d:prop><d:status>HTTP/1.1 {status} Result</d:status></d:propstat></d:response></d:multistatus>");
                    write!(socket,"HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",xml.len(),xml).unwrap();
                }
            });
            let cfg=Config{provider:Provider::Feishu,username:"demo".into(),href:Some(format!("{base}calendar/")),server:Some(base)};
            let result=fetch_month(&cfg,"test",NaiveDate::from_ymd_opt(2026,11,3).unwrap());
            if succeeds {assert_eq!(result.unwrap().events.len(),2);} else {assert!(result.is_err());}
            server.join().unwrap();
        }
    }

    #[test]
    fn caldav_collection_listing_reads_slash_objects_and_recurring_events() {
        use std::{io::Write,net::TcpListener};
        for succeeds in [true,false] {
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();
            let base=format!("http://{}/",listener.local_addr().unwrap());
            let server=std::thread::spawn(move||{
            serve_calendar_discovery(&listener);
                for index in 0..3 {
                    let (mut socket,_)=listener.accept().unwrap();
                    socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut buf=Vec::new();let mut chunk=[0u8;4096];
                    loop {
                        let n=socket.read(&mut chunk).unwrap();assert!(n>0);buf.extend_from_slice(&chunk[..n]);
                        if let Some(pos)=buf.windows(4).position(|s|s==b"\r\n\r\n") {
                            let head=String::from_utf8_lossy(&buf[..pos]);
                            let len=head.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                            if buf.len()>=pos+4+len {break;}
                        }
                    }
                    let request=String::from_utf8(buf).unwrap();
                    assert!(request.contains(if index==0 {"calendar-query"} else if index==1 {"PROPFIND"} else {"calendar-multiget"}));
                    if index != 1 { assert!(request.contains("<c:expand")); }
                    if index==2 {assert!(request.contains("<d:href>/calendar/item/</d:href>"));}
                    let data=if index==2 && succeeds {"BEGIN:VCALENDAR&#xD;&#xA;BEGIN:VEVENT&#xD;&#xA;UID:test&#xD;&#xA;SUMMARY:测试&#xD;&#xA;DTSTART:20261103T032000Z&#xD;&#xA;RRULE:FREQ=DAILY;COUNT=2&#xD;&#xA;END:VEVENT&#xD;&#xA;END:VCALENDAR"} else {""};
                    let status=if data.is_empty(){404}else{200};
                    let object=format!("<d:multistatus><d:response><d:href>/calendar/item/</d:href><d:propstat><d:prop><c:calendar-data>{data}</c:calendar-data></d:prop><d:status>HTTP/1.1 {status} Result</d:status></d:propstat></d:response></d:multistatus>");
                    let collection="<d:response><d:href>/calendar/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>";
                    let xml=if index==0 {format!("<d:multistatus>{collection}</d:multistatus>")} else if index==1 {format!("<d:multistatus>{collection}<d:response><d:href>/calendar/item/</d:href></d:response></d:multistatus>")} else {object};
                    write!(socket,"HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",xml.len(),xml).unwrap();
                }
            });
            let cfg=Config{provider:Provider::DingTalk,username:"demo".into(),href:Some(format!("{base}calendar/")),server:Some(base)};
            let result=fetch_month(&cfg,"test",NaiveDate::from_ymd_opt(2026,11,3).unwrap());
            if succeeds {assert_eq!(result.unwrap().events.len(),2);} else {assert!(result.is_err());}
            server.join().unwrap();
        }
    }

    #[test]
    fn month_grid_covers_leap_days_and_year_changes() {
        let feb=month_grid(NaiveDate::from_ymd_opt(2024,2,17).unwrap());
        assert_eq!(feb[0].weekday(),chrono::Weekday::Mon);
        assert!(feb.contains(&NaiveDate::from_ymd_opt(2024,2,29).unwrap()));
        assert_eq!(feb.iter().filter(|d|d.month()==2).count(),29);
        let dec=month_grid(NaiveDate::from_ymd_opt(2026,12,1).unwrap());
        assert_eq!(dec[41].year(),2027);
    }

    #[test]
    fn month_query_fetches_past_dates_and_excludes_the_next_month() {
        use std::{io::Write,net::TcpListener};
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
        let utc=|month,day|NaiveDate::from_ymd_opt(2024,month,day).unwrap().and_hms_opt(0,0,0).unwrap().and_local_timezone(chrono::Local).earliest().unwrap().with_timezone(&chrono::Utc);
        let start=utc(2,1);let end=utc(3,1);
        let ics=[("month-first",start),("leap-day",utc(2,29)),("next-month",end)].into_iter().map(|(uid,at)|format!("BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:月度安排\r\nDTSTART:{}\r\nEND:VEVENT\r\n",at.format("%Y%m%dT%H%M%SZ"))).collect::<String>();
        let server=std::thread::spawn(move||{
            serve_calendar_discovery(&listener);
            let (mut socket,_)=listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut buf=Vec::new();let mut chunk=[0u8;4096];
            loop {
                let n=socket.read(&mut chunk).unwrap();if n==0{break;}buf.extend_from_slice(&chunk[..n]);
                if let Some(pos)=buf.windows(4).position(|s|s==b"\r\n\r\n"){
                    let header=String::from_utf8_lossy(&buf[..pos]);
                    let len=header.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                    if buf.len()>=pos+4+len{break;}
                }
            }
            let xml=format!(r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR
{ics}END:VCALENDAR</c:calendar-data></d:prop></d:propstat></d:response></d:multistatus>"#);
            write!(socket,"HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",xml.len(),xml).unwrap();
            String::from_utf8(buf).unwrap()
        });
        let cfg=Config{provider:Provider::Feishu,username:"demo".into(),href:Some(format!("http://127.0.0.1:{port}/calendar/")),server:Some(format!("http://127.0.0.1:{port}/"))};
        let events=fetch_month(&cfg,"demo-password",NaiveDate::from_ymd_opt(2024,2,12).unwrap()).unwrap().events;
        assert_eq!(events.iter().map(|m|m.uid.as_str()).collect::<Vec<_>>(),["month-first","leap-day"]);
        let request=server.join().unwrap();
        assert!(request.contains(&format!("<c:time-range start=\"{}\" end=\"{}\"/>",start.format("%Y%m%dT%H%M%SZ"),end.format("%Y%m%dT%H%M%SZ"))));
    }

    #[test]
    fn automatic_sync_can_stop_resume_and_leave_manual_sync_available() {
        let (cmd,rx)=channel();let (tx,events)=channel();
        let worker=std::thread::spawn(move||run_with(rx,tx,Duration::from_millis(40),|cfg,_,ev|{
            ev.send(Event::Synced{config:cfg.clone(),events:vec![],at:String::new()}).unwrap();
        }));
        let config=Config{provider:Provider::Feishu,username:"demo".into(),href:None,server:None};
        cmd.send(Cmd::Configure(vec![config.clone()],false)).unwrap();
        events.recv_timeout(Duration::from_secs(1)).expect("新连接应同步一次");
        assert!(matches!(events.recv_timeout(Duration::from_millis(120)),Err(RecvTimeoutError::Timeout)));
        cmd.send(Cmd::SyncNow).unwrap();
        events.recv_timeout(Duration::from_secs(1)).expect("关闭自动同步后仍能手动同步");
        assert!(matches!(events.recv_timeout(Duration::from_millis(120)),Err(RecvTimeoutError::Timeout)));
        cmd.send(Cmd::Configure(vec![config.clone()],true)).unwrap();
        events.recv_timeout(Duration::from_secs(1)).expect("开启后应立即更新");
        events.recv_timeout(Duration::from_secs(1)).expect("开启后应继续定期更新");
        cmd.send(Cmd::Configure(vec![config],false)).unwrap();
        // 等工作线程处理关闭指令，并排空恰好已经完成的一次同步。
        let _=events.recv_timeout(Duration::from_millis(120));
        assert!(matches!(events.recv_timeout(Duration::from_millis(120)),Err(RecvTimeoutError::Timeout)));
        cmd.send(Cmd::SyncNow).unwrap();
        events.recv_timeout(Duration::from_secs(1)).expect("切换开关后手动同步仍可用");
        drop(cmd);worker.join().unwrap();
    }

    #[test]
    fn multiple_calendars_sync_even_when_one_fails() {
        let (cmd,rx)=channel();let (tx,events)=channel();
        let worker=std::thread::spawn(move||run_with(rx,tx,Duration::from_secs(60),|cfg,_,ev|{
            let event=if cfg.provider==Provider::Feishu {Event::Failed{config:cfg.clone(),message:"暂时不可用".into()}}
                else {Event::Synced{config:cfg.clone(),events:vec![],at:String::new()}};
            ev.send(event).unwrap();
        }));
        let feishu=Config{provider:Provider::Feishu,username:"demo".into(),href:None,server:None};
        let ding=Config{provider:Provider::DingTalk,..feishu.clone()};
        cmd.send(Cmd::Configure(vec![feishu,ding.clone()],false)).unwrap();
        assert!(matches!(events.recv_timeout(Duration::from_secs(1)).unwrap(),Event::Failed{..}));
        assert!(matches!(events.recv_timeout(Duration::from_secs(1)).unwrap(),Event::Synced{config,..} if config==ding));
        drop(cmd);worker.join().unwrap();
    }

    #[test]
    fn join_url_handles_absolute_path_and_relative() {
        assert_eq!(join_url("https://a.com/calendars/", "/calendars/u1/"), "https://a.com/calendars/u1/");
        assert_eq!(join_url("https://a.com/calendars/", "u2/"), "https://a.com/calendars/u2/");
        assert_eq!(join_url("https://a.com/dav", "u3/"), "https://a.com/dav/u3/");
        assert_eq!(join_url("https://a.com/", "https://b.com/x/"), "https://b.com/x/");
        assert_eq!(path_of("https://a.com/calendars/u1/"), "/calendars/u1/");
        assert_eq!(path_of("https://a.com"), "/");
    }

    #[test]
    fn propfind_207_picks_calendar_href() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
 <d:response><d:href>/calendars/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
 <d:response><d:href>/calendars/8f3c2d7a-1111-2222-3333-444455556666/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        let cal = pick_calendar(xml, "https://caldav.feishu.cn/calendars/").unwrap();
        assert_eq!(cal, "https://caldav.feishu.cn/calendars/8f3c2d7a-1111-2222-3333-444455556666/");
        assert_eq!(responses(xml).len(), 2);
    }

    #[test]
    fn calendar_home_set_is_followed() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/</d:href><d:propstat><d:prop><c:calendar-home-set><d:href>/dav/u1/</d:href></c:calendar-home-set></d:prop></d:propstat></d:response></d:multistatus>"#;
        assert_eq!(calendar_home(xml, "https://calendar.dingtalk.com/").as_deref(), Some("https://calendar.dingtalk.com/dav/u1/"));
    }

    #[test]
    fn basic_auth_is_rfc7617() {
        assert_eq!(basic_auth("Aladdin", "open sesame"), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert_eq!(basic_auth("用户", "密码"), "Basic 55So5oi3OuWvhueggQ==");
    }

    #[test]
    fn paste_splits_three_provider_formats() {
        let feishu = "服务器地址：https://caldav.feishu.cn/calendars/\n用户名：u_abc@feishu\n密码：p@ss:word1";
        assert_eq!(parse_paste(feishu), Some(("u_abc@feishu".into(), "p@ss:word1".into())));
        let ding = "账号 ding_001 密码 8f3k2j";
        assert_eq!(parse_paste(ding), Some(("ding_001".into(), "8f3k2j".into())));
        let wecom = "用户名=wecom_user 密码=Wx!2026";
        assert_eq!(parse_paste(wecom), Some(("wecom_user".into(), "Wx!2026".into())));
        assert_eq!(parse_paste("就是一个用户名"), None, "拆不出密码就不猜");
    }

    #[test]
    fn error_messages_are_human() {
        assert_eq!(Failure::status(Provider::Feishu, 401).message, "验证账号失败（401），请核对用户名和飞书提供的专用密码");
        assert_eq!(Failure::status(Provider::WeCom, 401).message, "验证账号失败（401），请核对用户名和企业微信提供的专用密码");
        assert_eq!(Failure::status(Provider::Exchange, 401).message, "验证账号失败（401），请核对用户名和密码");
        assert_eq!(Failure::status(Provider::Custom, 401).message, "验证账号失败（401），请核对用户名和密码");
        for provider in Provider::ALL {assert_eq!(Provider::parse(provider.key()),provider);}
        assert!(Provider::Exchange.exchange() && Provider::WeCom.exchange() && !Provider::Custom.exchange());
        assert!(Provider::Exchange.custom_server() && Provider::Exchange.server().is_empty());
        let saved=Config{provider:Provider::Exchange,username:"me@example.com".into(),href:None,server:Some("https://mail.example.com/".into())};
        assert_eq!(decode_config(&encode_config(&saved)).unwrap(),saved,"Exchange 账号的服务器地址要能保存和读回");
        assert_eq!(Failure::transport("timed out").message, "连不上服务器，检查网络");
        assert_eq!(Failure::status(Provider::DingTalk, 404).message, "服务器地址不对（404）");
    }

    #[test]
    fn report_requests_expanded_recurring_events() {
        let body = report_body(Provider::Feishu,"20261030T000000Z", "20261031T000000Z");
        assert!(body.contains("<c:expand start=\"20261030T000000Z\" end=\"20261031T000000Z\"/>"));
    }

    #[test]
    fn dedupe_key_uses_uid_and_start() {
        let a = crate::mail::Meeting { all_day:false, end:None, title: "周会".into(), start: 100, at: "01-01 09:00".into(), uid: "evt-1".into() };
        let b = crate::mail::Meeting { all_day:false, end:None, title: "周会（日历）".into(), start: 100, at: "01-01 09:00".into(), uid: "evt-1".into() };
        assert_eq!(dedupe_key(&a), dedupe_key(&b));
        let c = crate::mail::Meeting { all_day:false, end:None, title: "无 UID".into(), start: 100, at: "".into(), uid: String::new() };
        assert_eq!(dedupe_key(&c), "t100|无 UID");
    }

    #[test]
    fn expanded_recurring_instances_are_kept_separately() {
        let xml = r#"<c:calendar-data>BEGIN:VCALENDAR
BEGIN:VEVENT
UID:weekly
DTSTART:20261030T060000Z
END:VEVENT
BEGIN:VEVENT
UID:weekly
RECURRENCE-ID:20261106T060000Z
DTSTART:20261106T060000Z
END:VEVENT
END:VCALENDAR</c:calendar-data>"#;
        let events = events_from_report(xml);
        assert_eq!(events.len(), 2);
        assert_ne!(dedupe_key(&events[0]), dedupe_key(&events[1]));
    }

    #[test]
    fn report_extracts_and_dedupes_events() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
<d:response><d:href>/cal/x/1.ics</d:href><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR
BEGIN:VEVENT
UID:evt-9
SUMMARY:周会
DTSTART:20261030T060000Z
END:VEVENT
END:VCALENDAR</c:calendar-data></d:prop></d:propstat></d:response>
<d:response><d:href>/cal/y/2.ics</d:href><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR
BEGIN:VEVENT
UID:evt-9
SUMMARY:周会副本
DTSTART:20261030T060000Z
END:VEVENT
END:VCALENDAR</c:calendar-data></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        let ev = events_from_report(xml);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].uid, "evt-9");
        assert_eq!(ev[0].title, "周会");
        assert_eq!(ev[0].start, 1793340000); // 2026-10-30T06:00:00Z
    }

    /// 假 CalDAV：PROPFIND 回一个 uuid 日历，REPORT 回一条事件，Basic Auth 必须对。
    #[test]
    fn full_chain_against_local_server() {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut hits = Vec::new();
            for _ in 0..4 {
                let Ok((mut sock, _)) = listener.accept() else { return Vec::new() };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let Ok(n) = sock.read(&mut tmp) else { break };
                    if n == 0 { break; }
                    buf.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(pos) = text.find("\r\n\r\n") {
                        let head = &text[..pos];
                        let len = head
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let first = text.lines().next().unwrap_or("").to_string();
                hits.push(first.clone());
                let auth_ok = text.to_ascii_lowercase().contains("authorization: basic ");
                let body = if !auth_ok {
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                } else if first.starts_with("PROPFIND") {
                    let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/calendars/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response><d:response><d:href>/calendars/8f3c2d7a-1111-2222-3333-444455556666/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><cs:getctag xmlns:cs="http://calendarserver.org/ns/">ctag-7</cs:getctag></d:prop></d:propstat></d:response></d:multistatus>"#;
                    format!("HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", xml.len(), xml)
                } else if first.starts_with("REPORT") {
                    let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/calendars/u/1.ics</d:href><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR
BEGIN:VEVENT
UID:local-1
SUMMARY:本地评审
DTSTART:20261031T020000Z
END:VEVENT
END:VCALENDAR</c:calendar-data></d:prop></d:propstat></d:response></d:multistatus>"#;
                    let next = (chrono::Utc::now() + chrono::Duration::hours(1)).format("%Y%m%dT%H%M%SZ").to_string();
                    let xml = xml.replace("20261031T020000Z", &next);
                    format!("HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", xml.len(), xml)
                } else {
                    "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                };
                let _ = sock.write_all(body.as_bytes());
                let _ = sock.flush();
            }
            hits
        });

        let cfg = Config {
            provider: Provider::Feishu,
            username: "u1".into(),
            href: None,
            server: Some(format!("http://127.0.0.1:{port}/calendars/")),
        };
        let got = fetch(&cfg, "p1").expect("完整链路应成功");
        assert!(got.href.ends_with("/8f3c2d7a-1111-2222-3333-444455556666/"));
        assert_eq!(got.events.len(), 1);
        assert_eq!(got.events[0].uid, "local-1");

        // 即使日历内容未变，也要刷新不断向前移动的时间窗口。
        let again = fetch(&Config { href: Some(got.href.clone()), ..cfg.clone() }, "p1").unwrap();
        assert_eq!(again.events.len(), 1);
        let hits = server.join().unwrap();
        assert_eq!(hits.iter().filter(|h| h.starts_with("REPORT")).count(), 2);
    }
}

#[cfg(test)]
mod icloud_and_ics_tests {
    use super::*;

    #[test]
    fn new_providers_round_trip_and_custom_server_is_validated() {
        for p in Provider::ALL { assert_eq!(Provider::parse(p.key()), p); }
        assert!(Provider::ICloud.all_calendars() && !Provider::Feishu.all_calendars());
        assert_eq!(normalize_server("cloud.example.com/remote.php/dav").as_deref(), Some("https://cloud.example.com/remote.php/dav"));
        assert_eq!(normalize_server("webcal://p01-caldav.icloud.com/x").as_deref(), Some("https://p01-caldav.icloud.com/x"));
        assert!(normalize_server("http://example.com/dav").is_none(), "非本机地址必须 https");
        assert!(normalize_server("http://127.0.0.1:5232/").is_some());
        assert!(normalize_server("https://user:pw@example.com/").is_none());
        assert!(normalize_server("  ").is_none());
        let cfg = Config { provider: Provider::Custom, username: "me".into(), href: None, server: Some("https://dav.example.com/".into()) };
        let back = decode_config(&encode_config(&cfg)).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn icloud_may_move_to_partition_hosts_but_others_stay_same_origin() {
        let base = url::Url::parse("https://caldav.icloud.com/").unwrap();
        let part = url::Url::parse("https://p52-caldav.icloud.com/1234/calendars/home/").unwrap();
        assert!(same_service(Provider::ICloud, &part, &base));
        assert!(!same_service(Provider::ICloud, &url::Url::parse("https://evil.example.com/").unwrap(), &base));
        assert!(!same_service(Provider::ICloud, &url::Url::parse("http://p52-caldav.icloud.com/").unwrap(), &base));
        assert!(!same_service(Provider::Custom, &part, &url::Url::parse("https://dav.example.com/").unwrap()));
    }

    #[test]
    fn event_calendars_skip_reminder_lists_and_inbox() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
<d:response><d:href>/1234/calendars/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
<d:response><d:href>/1234/calendars/home/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
<c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>
<d:response><d:href>/1234/calendars/tasks/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
<c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>
<d:response><d:href>/1234/calendars/work/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop></d:propstat></d:response>
<d:response><d:href>/1234/calendars/inbox/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:schedule-inbox/></d:resourcetype></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        assert_eq!(event_calendars(xml, "https://p52-caldav.icloud.com/1234/calendars/"),
            vec!["https://p52-caldav.icloud.com/1234/calendars/home/".to_string(), "https://p52-caldav.icloud.com/1234/calendars/work/".to_string()]);
    }

    #[test]
    fn ics_file_events_expand_weekly_repeats() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Apple Inc.//macOS//EN\r\nBEGIN:VEVENT\r\nUID:apple-1\r\nSUMMARY:周会 & 复盘\r\nDTSTART:20261102T020000Z\r\nDTEND:20261102T030000Z\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:apple-2\r\nSUMMARY:体检\r\nDTSTART;VALUE=DATE:20261110\r\nDTEND;VALUE=DATE:20261111\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let lo = chrono::DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let hi = chrono::DateTime::parse_from_rfc3339("2027-01-01T00:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let events = events_from_ics(ics, lo, hi).unwrap();
        assert_eq!(events.iter().filter(|e| e.title == "周会 & 复盘").count(), 3);
        assert!(events.iter().any(|e| e.title == "体检" && e.all_day));
        assert!(events_from_ics("hello", lo, hi).is_err());
    }
}
