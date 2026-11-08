//! Windows 状态采样与经用户授权的通知读取。
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver};
use windows::core::{Result, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetLastInputInfo, LASTINPUTINFO,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{NotificationKinds, UserNotification};

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub volume: Option<(u32, bool)>,
    pub audio_endpoint: Option<String>,
    pub caps: bool,
    pub network: bool,
    pub network_kind: NetworkKind,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Observation {
    pub system: Snapshot,
    pub input: Option<crate::input::State>,
    pub fullscreen: Option<crate::display_layout::Bounds>,
}

/// 跨进程输入法查询和 COM 状态采样留在后台，只在状态改变时唤醒界面。
pub fn observe(ctx:eframe::egui::Context,initial:Snapshot,input:Option<crate::input::State>)->Receiver<Observation> {
    let(tx,rx)=mpsc::sync_channel(1);
    std::thread::Builder::new().name("daozai-system".into()).spawn(move||{
        let mut current=Observation{system:initial,input,fullscreen:None};
        let mut delivered=None;
        let mut sampled=crate::poll_clock::PollClock::default();
        let mut input_at=sampled;
        loop {
            let identity=crate::input::State::identity();
            let changed=identity.map(|state|(state.context,state.layout))!=current.input.map(|state|(state.context,state.layout));
            if changed || crate::input::switch_recent() || input_at.ready(std::time::Duration::from_secs(1)){
                current.input=crate::input::State::read();input_at=crate::poll_clock::PollClock::now();
            }
            if sampled.ready(std::time::Duration::from_millis(500)){
                current.system=Snapshot::read();current.fullscreen=fullscreen();sampled=crate::poll_clock::PollClock::now();
            }
            if delivered.as_ref()!=Some(&current) {
                match tx.try_send(current.clone()) {
                    Ok(())=>{delivered=Some(current.clone());ctx.request_repaint();},
                    Err(mpsc::TrySendError::Disconnected(_))=>return,
                    Err(mpsc::TrySendError::Full(_))=>{},
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }).ok();
    rx
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkKind { Wifi, Ethernet, #[default] Other }
impl Snapshot {
    pub fn read() -> Self {
        unsafe {
            let device=audio_device().ok();
            let audio_endpoint=device.as_ref().and_then(|d|d.GetId().ok()).and_then(|id|{
                let text=id.to_string().ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as _));
                text
            });
            let volume = device.and_then(|d|d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL,None).ok()).and_then(|a| {
                Some((
                    (a.GetMasterVolumeLevelScalar().ok()? * 100.0).round() as u32,
                    a.GetMute().ok()?.as_bool(),
                ))
            });
            let profile=windows::Networking::Connectivity::NetworkInformation::GetInternetConnectionProfile().ok();
            let network=profile.as_ref().and_then(|p|p.GetNetworkConnectivityLevel().ok()).is_some_and(|v|v==windows::Networking::Connectivity::NetworkConnectivityLevel::InternetAccess);
            let network_kind=profile.as_ref().map(|p| {
                if p.IsWlanConnectionProfile().unwrap_or(false){NetworkKind::Wifi}
                else if p.NetworkAdapter().ok().and_then(|a|a.IanaInterfaceType().ok())==Some(6){NetworkKind::Ethernet}
                else{NetworkKind::Other}
            }).unwrap_or_default();
            Self{volume,audio_endpoint,network,network_kind,caps:GetKeyState(0x14)&1!=0}
        }
    }
    pub fn changes(&self, old: &Self) -> Vec<(String, String)> {
        let mut out = vec![];
        if self.audio_endpoint == old.audio_endpoint && self.volume != old.volume {
            if let Some((v, mute)) = self.volume {
                out.push((
                    "System Volume".into(),
                    if mute {
                        "Muted".into()
                    } else {
                        format!("Volume {v}%")
                    },
                ));
            }
        }
        if self.caps != old.caps {
            out.push((
                "Caps Lock".into(),
                if self.caps { "On" } else { "Off" }.into(),
            ));
        }
        if self.network != old.network || self.network && self.network_kind != old.network_kind {
            let kind=if self.network {self.network_kind}else{old.network_kind};
            let status=if self.network {"Connected"}else{"Disconnected"};
            out.push((
                "Network".into(),
                match kind {NetworkKind::Wifi=>format!("Wi-Fi · {status}"),NetworkKind::Ethernet=>format!("Ethernet · {status}"),NetworkKind::Other=>status.into()},
            ));
        }
        out
    }
}
unsafe fn audio_device() -> Result<IMMDevice> {    thread_local! {static INITIALIZED:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
    INITIALIZED.with(|v| {
        if !v.replace(true) {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    });
    // 每个采样线程复用枚举器，但每次仍查询当前默认设备，保留热插拔响应。
    thread_local! {static ENUMERATOR:std::cell::RefCell<Option<IMMDeviceEnumerator>>=const{std::cell::RefCell::new(None)};}
    ENUMERATOR.with(|slot| {
        let mut slot=slot.borrow_mut();
        if slot.is_none(){*slot=Some(CoCreateInstance(&MMDeviceEnumerator,None,CLSCTX_ALL)?);}
        let result=slot.as_ref().unwrap().GetDefaultAudioEndpoint(eRender,eConsole);
        if result.is_err(){*slot=None;}
        result
    })
}
unsafe fn audio() -> Result<IAudioEndpointVolume> {audio_device()?.Activate(CLSCTX_ALL,None)}
/// 设置主音量（0..100）；同时解除静音，拖动音量条时立刻能听到。
#[cfg(windows)]
pub fn set_volume(percent: u32) {
    unsafe {
        if let Ok(a) = audio() {
            let _ = a.SetMasterVolumeLevelScalar((percent.min(100) as f32) / 100.0, std::ptr::null());
            let _ = a.SetMute(false, std::ptr::null());
        }
    }
}

/// 切换静音，返回切换后的静音状态（拿不到设备时返回 None）。
#[cfg(windows)]
pub fn toggle_mute() -> Option<bool> {
    unsafe {
        let a = audio().ok()?;
        let muted = a.GetMute().ok()?.as_bool();
        a.SetMute(!muted, std::ptr::null()).ok()?;
        Some(!muted)
    }
}

pub fn idle_seconds() -> f64 {
    unsafe {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            ..Default::default()
        };
        if GetLastInputInfo(&mut info).as_bool() {
            windows::Win32::System::SystemInformation::GetTickCount().wrapping_sub(info.dwTime)
                as f64
                / 1000.0
        } else {
            0.0
        }
    }
}
pub fn fullscreen() -> Option<crate::display_layout::Bounds> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == windows::Win32::System::Threading::GetCurrentProcessId() {
            return None;
        }
        // 桌面与资源管理器桌面窗口不能当成全屏游戏。
        let mut cls = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut cls);
        let cls = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
        if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return None;
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool()
        {
            return None;
        }
        let covers=r.left <= info.rcMonitor.left
            && r.top <= info.rcMonitor.top
            && r.right >= info.rcMonitor.right
            && r.bottom >= info.rcMonitor.bottom;
        covers.then_some(crate::display_layout::Bounds {left:info.rcMonitor.left,top:info.rcMonitor.top,right:info.rcMonitor.right,bottom:info.rcMonitor.bottom})
    }
}
pub(crate) fn is_browser_process(name:&str)->bool {
    ["chrome.exe","msedge.exe","firefox.exe","brave.exe","vivaldi.exe","opera.exe",
        "arc.exe","zen.exe","chromium.exe","librewolf.exe","floorp.exe","waterfox.exe",
        "duckduckgo.exe","360se.exe","360chrome.exe","qqbrowser.exe","sogouexplorer.exe","quark.exe"]
        .iter().any(|browser|name.eq_ignore_ascii_case(browser))
}

fn browser_overlaps_island(browser:RECT,island:RECT,dock:crate::model::Dock)->bool {
    if dock.is_side() {return window_covers_side(browser,island,dock);}
    // 窗口是 CANVAS_W 宽的透明画布，岛体中心固定在画布左侧 WIN_W/2 处（与水平位置、大小无关）。
    let scale=(island.right-island.left) as f32/crate::app::CANVAS_W;
    let center=island.left as f32+crate::app::WIN_W*scale/2.0;
    let half=crate::app::PW*scale/2.0;
    (browser.left as f32)<center+half && (browser.right as f32)>center-half
        && (browser.top as f32)<island.top as f32+crate::app::BAR_H*scale && browser.bottom>island.top
}

/// 左右停靠：画布是 SIDE_W × SIDE_H，岛体贴在画布的左 / 右边、竖直居中。
/// 前台窗口盖住岛所在的那一段屏幕边缘（例如最大化、全屏或贴边摆放）即视为遮挡。
fn window_covers_side(window:RECT,island:RECT,dock:crate::model::Dock)->bool {
    let scale=(island.right-island.left) as f32/crate::side_dock::SIDE_W;
    let cy=island.top as f32+crate::side_dock::SIDE_H*scale/2.0;
    let half=crate::app::PH*scale/2.0;
    let strip=crate::side_dock::SIDE_BAR_W*scale;
    let vertical=(window.top as f32)<cy+half && (window.bottom as f32)>cy-half;
    let horizontal=if dock==crate::model::Dock::Left {(window.left as f32)<island.left as f32+strip && window.right>island.left}
        else {(window.right as f32)>island.right as f32-strip && window.left<island.right};
    vertical && horizontal
}

pub fn browser_at_top(island:HWND,settings:&crate::model::Settings)->Option<bool> {
    unsafe {
        let foreground=GetForegroundWindow();
        if foreground.0.is_null(){return None;}
        let mut pid=0;
        GetWindowThreadProcessId(foreground,Some(&mut pid));
        // 设置、托盘和岛内输入沿用此前的避让状态，避免自身窗口引起反复切换。
        if pid==0 || pid==GetCurrentProcessId(){return None;}
        let mut browser_rect=RECT::default();let mut island_rect=RECT::default();
        if GetWindowRect(foreground,&mut browser_rect).is_err() || GetWindowRect(island,&mut island_rect).is_err(){return None;}
        if !browser_overlaps_island(browser_rect,island_rect,settings.dock){return Some(false);}
        let Ok(process)=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,false,pid) else{return Some(false)};
        let mut path=[0u16;1024];let mut length=path.len() as u32;
        let result=QueryFullProcessImageNameW(process,PROCESS_NAME_WIN32,PWSTR(path.as_mut_ptr()),&mut length);
        let _=CloseHandle(process);
        if result.is_err(){return Some(false);}
        let path=String::from_utf16_lossy(&path[..length as usize]);
        Some(avoidance_matches(&path,settings))
    }
}

pub enum Incoming {
    Revoked,
    Status(String),
    Message(String, String, String),
}
#[path="qq_toast_source.rs"]
mod qq_toast_source;

fn notification_identity(n:&UserNotification, number:u32, created:i64, known_qq:bool)->(String,String){
    if known_qq{return ("QQ".into(),"QQ".into());}
    let info=n.AppInfo().ok();
    let id=info.as_ref().and_then(|app|app.AppUserModelId().ok()).map(|id|id.to_string()).unwrap_or_default();
    if id.eq_ignore_ascii_case("QQ") || (id.is_empty() && qq_toast_source::matches(number,created)){
        return ("QQ".into(),"QQ".into());
    }
    let name=info.and_then(|app|app.DisplayInfo().ok()).and_then(|display|display.DisplayName().ok())
        .map(|name|name.to_string()).filter(|name|!name.trim().is_empty()).unwrap_or_else(||"Windows Notification".into());
    let id=if id.is_empty(){name.clone()}else{id};
    (name,id)
}
fn notification_body(n: &UserNotification) -> Result<String> {
    let binding = n
        .Notification()?
        .Visual()?
        .GetBinding(&HSTRING::from("ToastGeneric"))?;
    let texts = binding.GetTextElements()?;
    let mut parts = vec![];
    for i in 0..texts.Size()?.min(8) {
        parts.push(texts.GetAt(i)?.Text()?.to_string().chars().take(300).collect::<String>());
    }
    Ok(parts.join(" · "))
}
fn is_wechat_toast(app:&str,id:&str)->bool{
    ["微信","WeChat","Weixin"].iter().any(|name|app.trim().eq_ignore_ascii_case(name))
        || ["WeChat","Weixin","Tencent.WeChat","Tencent.Weixin"].iter().any(|name|id.eq_ignore_ascii_case(name))
}
/// 停止信号独立于消息队列，断开后结束等待，不再读取通知。
pub struct Notifications {
    incoming: Receiver<Incoming>,
    stop: mpsc::Sender<()>,
}
impl Notifications {
    pub fn try_recv(&self)->std::result::Result<Incoming,mpsc::TryRecvError>{self.incoming.try_recv()}
}
impl Drop for Notifications {fn drop(&mut self){let _=self.stop.send(());}}

#[derive(Clone,Copy,PartialEq,Eq)]
struct ToastVersion { created:i64, qq_body:Option<u64> }
impl ToastVersion {
    fn new(created:i64,qq_body:Option<&str>)->Self{
        Self{created,qq_body:qq_body.map(|body|{let mut hash=std::collections::hash_map::DefaultHasher::new();body.hash(&mut hash);hash.finish()})}
    }
}
#[derive(Default)]
struct NotificationBaseline { previous: Option<HashMap<u32,ToastVersion>> }
impl NotificationBaseline {
    fn unchanged_other(&self,id:u32,created:i64)->Option<ToastVersion>{
        self.previous.as_ref()?.get(&id).copied().filter(|old|old.created==created && old.qq_body.is_none())
    }
    fn changed(&self,id:u32,version:ToastVersion)->bool{
        self.previous.as_ref().is_some_and(|old|old.get(&id)!=Some(&version))
    }
}

fn qq_channel_status()->String{
    use windows::UI::Notifications::{NotificationSetting,ToastNotificationManager};
    match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from("QQ")).and_then(|notifier|notifier.Setting()){
        Ok(NotificationSetting::Enabled)=>"QQ system notification channel available".into(),
        Ok(NotificationSetting::DisabledForApplication)=>"QQ system notifications are off; turn them on in Windows notification settings, then reconnect".into(),
        Ok(NotificationSetting::DisabledForUser)=>"Windows system notifications are off; turn them on, then reconnect".into(),
        Ok(NotificationSetting::DisabledByGroupPolicy)=>"Notifications blocked by system policy".into(),
        _=>"QQ system notification channel not ready; check QQ installation and notification settings, then reconnect".into(),
    }
}

/// 必须由窗口线程发起权限请求，后台仅读取新增或更新的通知。
pub fn connect(ctx:eframe::egui::Context,qq_only:bool) -> Result<Notifications> {
    let listener = UserNotificationListener::Current()?;
    let request = listener.RequestAccessAsync()?;
    let (tx, rx) = mpsc::sync_channel(32);
    let (stop, stopped)=mpsc::channel();
    std::thread::Builder::new().name("daozai-notifications".into()).spawn(move || {
        let send=|event|{
            let result=tx.send(event);
            if result.is_ok(){ctx.request_repaint();}
            result.is_ok()
        };
        let run = || -> Result<()> {
            unsafe {windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED)}?;
            struct Apartment;
            impl Drop for Apartment {fn drop(&mut self){unsafe{windows::Win32::System::WinRT::RoUninitialize();}}}
            let _apartment=Apartment;
            if request.get()? != UserNotificationListenerAccessStatus::Allowed {
                send(Incoming::Revoked);
                send(Incoming::Status("Windows hasn't granted notification access; you can change this in system settings.".into()));
                return Ok(());
            }
            let listener = UserNotificationListener::Current()?;
            let qq_status=qq_channel_status();
            let mut baseline=NotificationBaseline::default();
            let mut last_status=String::new();
            loop {
                if stopped.try_recv()!=Err(mpsc::TryRecvError::Empty){return Ok(());}
                if listener.GetAccessStatus()? != UserNotificationListenerAccessStatus::Allowed {
                    send(Incoming::Revoked);
                    send(Incoming::Status("Notification permission has been turned off".into()));
                    return Ok(());
                }
                let items=listener.GetNotificationsAsync(NotificationKinds::Toast)?.get()?;
                let mut current=HashMap::new();
                let mut messages=Vec::new();
                for i in 0..items.Size()? {
                    let Ok(n)=items.GetAt(i) else{continue};
                    let (Ok(number),Ok(created))=(n.Id(),n.CreationTime()) else{continue};
                    let created=created.UniversalTime;
                    if let Some(old)=baseline.unchanged_other(number,created){current.insert(number,old);continue;}
                    let known_qq=baseline.previous.as_ref().and_then(|old|old.get(&number)).is_some_and(|old|old.created==created && old.qq_body.is_some());
                    let (app,id)=notification_identity(&n,number,created,known_qq);
                    // 微信只走任务栏事件，即使客户端同时发送系统通知也不读取其正文。
                    if (qq_only && id!="QQ") || is_wechat_toast(&app,&id){current.insert(number,ToastVersion::new(created,None));continue;}
                    let qq=id=="QQ";
                    // QQ 可能原地更新同一条通知；其他应用的启动历史不需要读取正文。
                    let body=if qq || baseline.previous.is_some(){notification_body(&n).unwrap_or_default()}else{String::new()};
                    let version=ToastVersion::new(created,qq.then_some(body.as_str()));
                    current.insert(number,version);
                    if baseline.changed(number,version){
                        let body=if qq && body.trim().is_empty(){"New message".into()}else{body};
                        if !body.trim().is_empty(){messages.push((created,app,id,body));}
                    }
                }
                baseline.previous=Some(current);
                messages.sort_by_key(|item|item.0);
                for (_,app,id,body) in messages {
                    if !send(Incoming::Message(app,id,body)){return Ok(());}
                }
                let status=if qq_only{format!("Listening to QQ only · {qq_status}")}else{format!("Listening to Windows notifications · {} visible\n{qq_status}",items.Size()?)};
                if status!=last_status {
                    if !send(Incoming::Status(status.clone())){return Ok(());}
                    last_status=status;
                }
                if stopped.recv_timeout(std::time::Duration::from_secs(1))!=Err(mpsc::RecvTimeoutError::Timeout){return Ok(());}
            }
        };
        if let Err(e) = run() {
            send(Incoming::Status(format!("Windows doesn't allow reading notifications yet: {}. Please check notification access and app identity.",e.code())));
        }
    }).map_err(|_|windows::core::Error::from_win32())?;
    Ok(Notifications{incoming:rx,stop})
}

pub fn pick_files() -> Result<Vec<std::path::PathBuf>> {
    pick_files_for("Add Files to Shelf",true)
}

pub fn pick_files_for(title:&str,multiple:bool) -> Result<Vec<std::path::PathBuf>> {
    unsafe {
        use windows::Win32::UI::Shell::*;
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL)?;
        let mut options=FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST;
        if multiple{options|=FOS_ALLOWMULTISELECT;}
        dialog.SetOptions(options)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        if let Err(e) = dialog.Show(None) {
            if e.code().0 as u32 == 0x800704c7 {
                return Ok(vec![]);
            }
            return Err(e);
        }
        let items = dialog.GetResults()?;
        let mut out = vec![];
        for i in 0..items.GetCount()? {
            let ptr = items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let text = ptr.to_string()?;
            windows::Win32::System::Com::CoTaskMemFree(Some(ptr.0 as _));
            out.push(text.into());
        }
        Ok(out)
    }
}
/// 构建资源管理器可识别的文件数据对象。
fn file_data_object(path: &std::path::Path) -> Result<windows::Win32::System::Com::IDataObject> {
    unsafe {
        use windows::Win32::UI::Shell::*;
        let wide: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let pidl = ILCreateFromPathW(windows::core::PCWSTR(wide.as_ptr()));
        if pidl.is_null() {
            return Err(windows::core::Error::from_win32());
        }
        let result = (|| {
            let mut child = std::ptr::null_mut();
            let folder: IShellFolder = SHBindToParent(pidl, Some(&mut child))?;
            folder.GetUIObjectOf(None, &[child as *const _], None)
        })();
        ILFree(Some(pidl));
        result
    }
}
/// 原生拖出只允许复制，取消拖放不会改变置物架与原文件。
pub fn drag_file(path: &std::path::Path) -> Result<()> {
    unsafe {
        use windows::Win32::System::Ole::{OleInitialize, OleUninitialize, DROPEFFECT_COPY};
        use windows::Win32::UI::Shell::SHDoDragDrop;
        let initialized = OleInitialize(None).is_ok();
        let result = file_data_object(path)
            .and_then(|data| SHDoDragDrop(None, &data, None, DROPEFFECT_COPY).map(|_| ()));
        if initialized {
            OleUninitialize();
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notifications_skip_history_but_detect_reused_ids_and_updated_content(){
        let mut baseline=NotificationBaseline::default();
        let first=ToastVersion::new(10,Some("第一条"));
        assert!(!baseline.changed(1,first));
        baseline.previous=Some(HashMap::from([(1,first),(2,ToastVersion::new(10,None))]));
        assert!(!baseline.changed(1,first));
        assert!(baseline.changed(1,ToastVersion::new(10,Some("第二条"))));
        assert!(baseline.changed(1,ToastVersion::new(11,Some("第一条"))));
        assert!(baseline.unchanged_other(2,10).is_some());
        assert!(baseline.unchanged_other(1,10).is_none());
        assert!(baseline.unchanged_other(2,11).is_none());
        baseline.previous=Some(HashMap::new());assert!(baseline.changed(1,first));
    }
    #[test]
    fn notifications_disconnect_signals_stop_without_waiting_for_new_messages(){
        let (_tx,incoming)=mpsc::sync_channel(32);let (stop,stopped)=mpsc::channel();
        drop(Notifications{incoming,stop});
        assert_eq!(stopped.try_recv(),Ok(()));
    }
    #[test]
    fn browser_minimal_recognizes_browsers_without_matching_electron_apps(){
        for name in ["chrome.exe","MSEDGE.EXE","firefox.exe","brave.exe","zen.exe","QQBrowser.exe","quark.exe","Quark.exe","QUARK.EXE"] {
            assert!(is_browser_process(name),"应识别浏览器 {name}");
        }
        for name in ["Codex.exe","Code.exe","msedgewebview2.exe","explorer.exe","chrome-helper.exe","quark_proxy.exe","quark_pwa_launcher.exe","quark_host_client.exe","quark_swap_util.exe","QuarkUpdaterSetup.exe",""] {
            assert!(!is_browser_process(name),"不能将 {name} 当作浏览器");
        }
    }
    #[test]
    fn browser_minimal_checks_the_actual_top_overlap_on_the_island_monitor(){
        let island=RECT{left:560,top:0,right:1360,bottom:238};
        for browser in [RECT{left:0,top:-8,right:1920,bottom:1200},RECT{left:0,top:0,right:960,bottom:1150},RECT{left:700,top:10,right:1500,bottom:800}] {
            assert!(browser_overlaps_island(browser,island,crate::model::Dock::Top));
        }
        for browser in [RECT{left:100,top:150,right:1700,bottom:1100},RECT{left:1920,top:0,right:3840,bottom:1200},RECT{left:0,top:0,right:600,bottom:1000},RECT{left:0,top:-1200,right:1920,bottom:0}] {
            assert!(!browser_overlaps_island(browser,island,crate::model::Dock::Top));
        }
    }
    #[test]
    fn side_dock_avoidance_checks_the_covered_screen_edge(){
        use crate::model::Dock;
        // 1920×1080 屏幕，100% 缩放：左侧画布 0..500，竖直居中（240..840），岛体中心 y=540。
        let left=RECT{left:0,top:240,right:500,bottom:840};
        let right=RECT{left:1420,top:240,right:1920,bottom:840};
        let maximized=RECT{left:-8,top:-8,right:1928,bottom:1048};
        assert!(browser_overlaps_island(maximized,left,Dock::Left) && browser_overlaps_island(maximized,right,Dock::Right));
        let left_half=RECT{left:0,top:0,right:960,bottom:1040};
        assert!(browser_overlaps_island(left_half,left,Dock::Left) && !browser_overlaps_island(left_half,right,Dock::Right),"贴左半屏只遮住左侧");
        let floating=RECT{left:300,top:100,right:1600,bottom:900};
        assert!(!browser_overlaps_island(floating,left,Dock::Left) && !browser_overlaps_island(floating,right,Dock::Right),"没碰到边缘的窗口不避让");
        let top_strip=RECT{left:0,top:0,right:1920,bottom:300};
        assert!(!browser_overlaps_island(top_strip,left,Dock::Left),"没盖到岛所在的那一段边缘");
    }
    #[test]
    fn network_connection_uses_connected_wording() {
        let old=Snapshot{volume:None,audio_endpoint:None,caps:false,network:false,network_kind:NetworkKind::Other};
        let mut connected=old.clone();connected.network=true;
        assert_eq!(connected.changes(&old),vec![("网络连接".into(),"网络已连接".into())]);
    }
    #[test]
    fn network_switches_transport_and_keeps_the_disconnected_type() {
        let wifi=Snapshot{volume:None,audio_endpoint:None,caps:false,network:true,network_kind:NetworkKind::Wifi};
        let mut cable=wifi.clone();cable.network_kind=NetworkKind::Ethernet;
        assert_eq!(cable.changes(&wifi),vec![("网络连接".into(),"以太网 · 网络已连接".into())]);
        let mut off=cable.clone();off.network=false;off.network_kind=NetworkKind::Other;
        assert_eq!(off.changes(&cable),vec![("网络连接".into(),"以太网 · 网络已断开".into())]);
        assert_eq!(wifi.changes(&off),vec![("网络连接".into(),"Wi-Fi · 网络已连接".into())]);
    }
    #[test]
    fn switching_audio_endpoints_is_not_a_volume_adjustment() {
        let old=Snapshot{volume:Some((20,false)),audio_endpoint:Some("speakers".into()),caps:false,network:true,network_kind:NetworkKind::Other};
        let mut new=old.clone();
        new.audio_endpoint=Some("headphones".into());new.volume=Some((80,false));new.caps=true;
        assert_eq!(new.changes(&old),vec![("大写锁定".into(),"已开启".into())]);
        let mut adjusted=new.clone();adjusted.volume=Some((60,false));
        assert_eq!(adjusted.changes(&new),vec![("系统音量".into(),"音量 60%".into())]);
        adjusted.volume=Some((60,true));
        assert_eq!(adjusted.changes(&new),vec![("系统音量".into(),"已静音".into())]);
        assert!(!old.changes(&new).iter().any(|(title,_)|title=="系统音量"));
    }

    #[test]
    fn dragged_file_exposes_native_file_drop_format() {
        unsafe {
            use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
            use windows::Win32::System::Ole::{OleInitialize, OleUninitialize, CF_HDROP};
            let initialized = OleInitialize(None).is_ok();
            let path =
                std::env::temp_dir().join(format!("daozai-drag-test-{}.txt", std::process::id()));
            std::fs::write(&path, "test").unwrap();
            {
                let data = file_data_object(&path).unwrap();
                let format = FORMATETC {
                    cfFormat: CF_HDROP.0,
                    dwAspect: DVASPECT_CONTENT.0,
                    lindex: -1,
                    tymed: TYMED_HGLOBAL.0 as u32,
                    ..Default::default()
                };
                assert!(data.QueryGetData(&format).is_ok());
            }
            std::fs::remove_file(path).unwrap();
            if initialized {
                OleUninitialize();
            }
        }
    }
}

fn avoidance_matches(path:&str,settings:&crate::model::Settings)->bool {
    settings.browser_minimal && (crate::avoidance::contains_app(&settings.avoid_apps,path)
        || settings.avoid_browsers && std::path::Path::new(path).file_name().and_then(|name|name.to_str()).is_some_and(is_browser_process))
}

pub(crate) fn avoidance_windows()->std::result::Result<Vec<crate::avoidance::Candidate>,String> {
    use windows::Win32::Foundation::{BOOL,LPARAM};
    unsafe extern "system" fn visit(hwnd:HWND,param:LPARAM)->BOOL {
        if !IsWindowVisible(hwnd).as_bool() || GetWindowLongPtrW(hwnd,GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0!=0 {return BOOL(1);}
        let mut cloaked=0u32;
        let _=windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(hwnd,windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,(&mut cloaked as *mut u32).cast(),4);
        if cloaked!=0{return BOOL(1);}
        let mut title=[0u16;1024];let len=GetWindowTextW(hwnd,&mut title);
        if len==0{return BOOL(1);}
        let mut pid=0;GetWindowThreadProcessId(hwnd,Some(&mut pid));
        if pid==GetCurrentProcessId(){return BOOL(1);}
        let Ok(process)=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,false,pid)else{return BOOL(1)};
        let mut path=[0u16;32768];let mut length=path.len() as u32;
        let result=QueryFullProcessImageNameW(process,PROCESS_NAME_WIN32,PWSTR(path.as_mut_ptr()),&mut length);
        let _=CloseHandle(process);
        if result.is_err(){return BOOL(1);}
        let path=String::from_utf16_lossy(&path[..length as usize]);
        let items=&mut *(param.0 as *mut Vec<crate::avoidance::Candidate>);
        if items.iter().any(|a|crate::avoidance::same_app(&a.path,&path)){return BOOL(1);}
        let filename=std::path::Path::new(&path).file_name().and_then(|s|s.to_str()).unwrap_or("");
        let name=executable_description(&path).unwrap_or_else(||filename.trim_end_matches(".exe").to_string());
        items.push(crate::avoidance::Candidate{browser:is_browser_process(filename),path,name,title:String::from_utf16_lossy(&title[..len as usize])});
        BOOL(1)
    }
    let mut items:Vec<crate::avoidance::Candidate>=Vec::new();
    unsafe{EnumWindows(Some(visit),LPARAM(&mut items as *mut _ as isize))}.map_err(|_|"Can't read the app list; refresh and try again".to_string())?;
    items.sort_by_key(|a|a.name.to_lowercase());
    Ok(items)
}

fn executable_description(path:&str)->Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW,GetFileVersionInfoW,VerQueryValueW};
    let path:Vec<u16>=path.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let size=GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()),None);if size==0{return None;}
        let mut bytes=vec![0u8;size as usize];GetFileVersionInfoW(PCWSTR(path.as_ptr()),0,size,bytes.as_mut_ptr().cast()).ok()?;
        let mut ptr=std::ptr::null_mut();let mut len=0;
        if !VerQueryValueW(bytes.as_ptr().cast(),windows::core::w!("\\VarFileInfo\\Translation"),&mut ptr,&mut len).as_bool() || len<4{return None;}
        let translation=std::slice::from_raw_parts(ptr as *const u16,2);
        let key:Vec<u16>=format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription",translation[0],translation[1]).encode_utf16().chain(Some(0)).collect();
        if !VerQueryValueW(bytes.as_ptr().cast(),PCWSTR(key.as_ptr()),&mut ptr,&mut len).as_bool() || len<=1{return None;}
        let name=String::from_utf16_lossy(std::slice::from_raw_parts(ptr as *const u16,len as usize)).trim_end_matches('\0').trim().to_string();
        (!name.is_empty()).then_some(name)
    }
}

#[cfg(test)]
mod avoidance_tests {
    use super::*;
    #[test]
    fn avoidance_respects_master_browser_switch_and_custom_app_identity() {
        let mut settings=crate::model::Settings::default();
        settings.avoid_apps=vec![(r"D:\Apps\Editor.exe".into(),"编辑器".into())];
        assert!(!avoidance_matches(r"D:\Apps\Editor.exe",&settings));
        settings.browser_minimal=true;
        assert!(avoidance_matches(r"D:\Quark\quark.exe",&settings));
        assert!(avoidance_matches(r"d:\apps\EDITOR.EXE",&settings));
        assert!(!avoidance_matches(r"C:\Other\Editor.exe",&settings));
        settings.avoid_browsers=false;
        assert!(!avoidance_matches(r"D:\Quark\quark.exe",&settings));
        assert!(avoidance_matches(r"D:\Apps\Editor.exe",&settings));
        settings.avoid_apps.clear();
        assert!(!avoidance_matches(r"D:\Apps\Editor.exe",&settings));
    }
    #[test]
    fn avoidance_window_list_excludes_this_process_and_deduplicates_apps() {
        let entries=avoidance_windows().unwrap();
        let current=std::env::current_exe().unwrap().to_string_lossy().into_owned();
        for (index,app) in entries.iter().enumerate() {
            assert!(!crate::avoidance::same_app(&app.path,&current));
            assert!(!app.name.is_empty() && !app.title.is_empty());
            assert!(!entries[..index].iter().any(|previous|crate::avoidance::same_app(&previous.path,&app.path)));
        }
        let notepad=std::path::Path::new(r"C:\Windows\System32\notepad.exe");
        if notepad.exists(){assert!(executable_description(notepad.to_str().unwrap()).is_some());}
    }
}
