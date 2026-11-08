#![cfg_attr(windows, windows_subsystem = "windows")]
//! Windows 外壳：无边框透明窗口 + 托盘 + 系统数据，界面全部交给 `daozai::app::App`。

use eframe::egui::{self, Color32, Pos2};
use daozai::app::{self, App, Cmd, Env, Options};
use daozai::smtc;
use daozai::poll_clock::PollClock;
use std::time::{Duration, Instant};

mod diagnostics;
mod graphics;

struct Shell {
    app: App,
    t0: Instant,
    #[cfg(windows)]
    frame_at: Instant,
    media: smtc::Shared,
    seek_rx: Option<std::sync::mpsc::Receiver<bool>>,
    media_control:Option<smtc::ControlWorker>,
    _resource_monitor:Option<daozai::resources::Monitor>,
    resource_at:PollClock,
    guard_notified:bool,
    media_enabled: smtc::Poller,
    placed: bool,
    placement_at: PollClock,
    placement_scale: f32,
    /// 上次定位时的目标屏幕与水平位置；变化后立即重新定位，不等下一次轮询。
    placement_key: (String, i32, daozai::model::Dock),
    win_x: f32,
    sys: (Option<u8>, bool, PollClock),
    #[cfg(windows)]
    system_state: daozai::system::Snapshot,
    #[cfg(windows)]
    system_ready: bool,
    #[cfg(windows)]
    system_rx: std::sync::mpsc::Receiver<daozai::system::Observation>,
    fullscreen: Option<daozai::display_layout::Bounds>,
    input_state:daozai::input::Monitor,
    #[cfg(windows)]
    input_keys:Option<daozai::input::Shortcuts>,
    #[cfg(windows)]
    incoming: Option<daozai::system::Notifications>,
    #[cfg(windows)]
    wechat: Option<daozai::wechat::Listener>,
    #[cfg(windows)]
    wechat_retry: Option<Instant>,
    #[cfg(windows)]
    opening_messenger:Option<(app::Unread,Instant)>,
    /// 后台线程正在通过托盘按钮唤起微信。
    #[cfg(windows)]
    tray_opening:Option<(app::Unread,std::sync::mpsc::Receiver<Result<(),String>>)>,
    #[cfg(windows)]
    hidden: bool,
    /// 毛玻璃皮肤的背景亮度采样（仅毛玻璃皮肤运行）。
    #[cfg(windows)]
    backdrop: Option<daozai::backdrop::Sampler>,
    #[cfg(windows)]
    native: Option<daozai::native::Window>,
    #[cfg(windows)]
    tray: Option<daozai::tray::Tray>,
    #[cfg(windows)]
    autostart: bool,
    /// v0.5.4：蓝牙音频设备（后台线程推送）与音量回声的静默窗口。
    #[cfg(windows)]
    bt_rx: Option<daozai::bt::Service>,
    bt: Vec<daozai::bt::Device>,
    bt_ready: bool,
    volume_quiet: PollClock,
    /// v0.5.5：邮件收信服务（工作线程），以及上次同步给它的账号/开关快照。
    mail: daozai::mail::Service,
    mail_sig: (bool, u32, Vec<daozai::mail::Account>),
    /// v0.5.6：日程（CalDAV）服务与上次下发的配置。
    cal: daozai::calendar::Service,
    cal_sig: (Vec<daozai::calendar::Config>, bool),
    quota: daozai::quota::Service,
    quota_sig: Option<(bool,Vec<daozai::quota::Account>)>,
    quota_at: PollClock,
}

impl Shell {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        diagnostics::graphics(cc);
        diagnostics::startup("Load local config");
        let mut app = App::new(&cc.egui_ctx, Options::default());
        #[cfg(feature="record")] let quota_demo=std::env::var("DAOZAI_QUOTA_DEMO").is_ok();
        #[cfg(not(feature="record"))] let quota_demo=false;
        diagnostics::startup("Initialize quota config");
        if !quota_demo{app.quota_initialize();}
        diagnostics::startup("Start clipboard history");
        {let ctx=cc.egui_ctx.clone();daozai::clipboard::watch(app.clipboard.clone(),move||ctx.request_repaint());}
        if std::env::args().any(|arg|arg=="--settings"){app.open_settings();}
        if std::env::args().any(|arg|arg=="--calendar"){app.open_calendar(None);}
        #[cfg(feature="record")]
        if std::env::var("DAOZAI_MAIL_DEMO").is_ok(){app.mail_demo();}
        #[cfg(feature="record")]
        if std::env::var("DAOZAI_CAL_DEMO").is_ok(){app.calendar_demo();}
        #[cfg(feature="record")]
        if let Ok(mode)=std::env::var("DAOZAI_QUOTA_DEMO"){app.quota_demo(&mode);}
        diagnostics::startup("Initialize media and tray");
        let media = smtc::shared();
        let media_enabled=smtc::spawn_poller(media.clone(),cc.egui_ctx.clone());
        media_enabled.store(app.settings.visible_pages[0],std::sync::atomic::Ordering::Relaxed);
        #[cfg(windows)]
        let autostart = daozai::tray::autostart_enabled();
        #[cfg(windows)]
        let tray = daozai::tray::Tray::start(daozai::tray::State {
            pet: app.settings.pet,
            minimal: app.settings.minimal,
            autostart,
        });
        diagnostics::startup("Initialize notification listener");
        #[cfg(windows)]
        let incoming=if app.settings.read_notifications {match daozai::system::connect(cc.egui_ctx.clone(),app.settings.qq_only){Ok(rx)=>Some(rx),Err(e)=>{app.listener_status=format!("Windows can't read notifications: {}",e.code());None}}}else{None};
        // 音频、网络与输入法查询由后台完成，避免首帧等待系统服务。
        diagnostics::startup("Create background sampling and window");
        let initial_input=None;
        #[cfg(windows)] let system_state=daozai::system::Snapshot::default();
        #[cfg(windows)] let system_rx=daozai::system::observe(cc.egui_ctx.clone(),system_state.clone(),initial_input);
        // 岛体大小只作用于主窗口，在首帧之前生效，避免启动时先按 100% 闪一下。
        #[cfg(windows)] eframe::set_root_viewport_scale(app.settings.island_scale());
        Self {
            app,
            t0: Instant::now(),
            #[cfg(windows)]
            frame_at: Instant::now(),
            media,
            seek_rx: None,
            media_control:None,
            _resource_monitor:daozai::resources::Monitor::start(daozai::model::config_dir().join("diagnostics")),
            resource_at:PollClock::default(),
            guard_notified:false,
            media_enabled,
            placed: false,
            placement_at: PollClock::default(),
            placement_scale: 0.0,
            placement_key: (String::new(), 0, daozai::model::Dock::Top),
            win_x: 0.0,
            sys: (None, false, PollClock::default()),
            #[cfg(windows)]
            system_state,
            #[cfg(windows)] system_ready:false,
            #[cfg(windows)] system_rx,
            fullscreen:None,
            input_state:daozai::input::Monitor::new(initial_input),
            #[cfg(windows)] input_keys:daozai::input::Shortcuts::start(),
            #[cfg(windows)] incoming,
            #[cfg(windows)] wechat:None,
            #[cfg(windows)] wechat_retry:None,
            #[cfg(windows)] opening_messenger:None,
            #[cfg(windows)] tray_opening:None,
            #[cfg(windows)] hidden:false,
            #[cfg(windows)] backdrop:None,
            #[cfg(windows)]
            native: daozai::native::Window::new(cc),
            #[cfg(windows)]
            tray,
            #[cfg(windows)]
            autostart,
            #[cfg(windows)]
            bt_rx: None,
            bt: Vec::new(),
            bt_ready: false,
            volume_quiet: PollClock::default(),
            mail: daozai::mail::Service::start(),
            mail_sig: (false, 0, Vec::new()),
            cal: daozai::calendar::Service::start(),
            cal_sig: (Vec::new(), true),
            quota: daozai::quota::Service::start(), quota_sig: None, quota_at: PollClock::default(),
        }
    }

    #[cfg(windows)]
    fn poll_tray(&mut self) {
        use daozai::tray::{self, Event};
        let events = self.tray.as_ref().map(|t| t.poll()).unwrap_or_default();
        for e in events {
            match e {
                Event::OpenSettings=>self.app.open_settings(),
                Event::OpenCalendar=>self.app.open_calendar(None),
                Event::TogglePet => {
                    self.app.set_pet(!self.app.settings.pet);
                    self.app.say(if self.app.settings.pet { "Mascot on" } else { "Mascot off" });
                }
                Event::ToggleMinimal => {
                    self.app.set_minimal(!self.app.settings.minimal);
                    self.app.say(if self.app.settings.minimal { "Minimal Notch on" } else { "Minimal Notch off" });
                }
                Event::ToggleAutostart => {
                    let want = !self.autostart;
                    match tray::set_autostart(want) {
                        Ok(()) => {
                            self.autostart = want;
                            self.app.say(if want { "Launch at startup enabled" } else { "Launch at startup disabled" });
                        }
                        Err(e) => self.app.say(e),
                    }
                }
            }
        }
        if let Some(t) = &self.tray {
            t.sync(tray::State { pet: self.app.settings.pet, minimal: self.app.settings.minimal, autostart: self.autostart });
        }
    }

    fn env(&mut self, ctx: &egui::Context) -> Env {
        let (clock, today, date) = local_now();
        // 电量变化很慢，每 5 秒读一次就够
        if self.sys.2.ready(Duration::from_secs(5)) {
            let (b, c) = read_power();
            if self.app.settings.show_battery && self.sys.0.is_some() && self.sys.1!=c && self.app.settings.system_hud {self.app.alert(if c {"Power Connected"}else{"On Battery Power"},b.map(|v|format!("Battery {v}%")).unwrap_or_else(||"Power status updated".into()),5,false);}
            if self.app.settings.show_battery && b.is_some_and(|v|v<=20) && !c && self.sys.0.is_some_and(|v|v>20) {self.app.alert("Low Battery",format!("{}% left · please plug in",b.unwrap()),5,true);}
            self.sys = (b, c, PollClock::now());
        }
        let ppp = ctx.pixels_per_point();
        let origin = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min).unwrap_or(Pos2::new(self.win_x, 0.0));
        #[cfg(windows)]
        let origin=self.native.as_ref().and_then(|w|w.logical_origin(ppp)).unwrap_or(origin);
        let cursor = cursor_logical(ppp).map(|(x, y)| Pos2::new(x - origin.x, y - origin.y));
        // 所在屏幕换算到窗口逻辑坐标，供通知详情等浮层避开屏幕边缘。
        #[cfg(windows)]
        let screen=daozai::display_layout::selected(&self.app.displays,&self.app.settings.display_id).map(|d|{
            let b=d.bounds;
            egui::Rect::from_min_max(Pos2::new(b.left as f32/ppp-origin.x,b.top as f32/ppp-origin.y),Pos2::new(b.right as f32/ppp-origin.x,b.bottom as f32/ppp-origin.y))
        });
        #[cfg(not(windows))]
        let screen=None;
        #[cfg(windows)]
        let (backdrop,glass_blur)=(self.backdrop.as_ref().and_then(|b|b.color()),self.native.as_ref().is_some_and(|w|w.blur_active()));
        #[cfg(not(windows))]
        let (backdrop,glass_blur)=(None,false);
        Env {
            t: self.t0.elapsed().as_secs_f64(),
            clock,
            today,
            date,
            battery: self.sys.0,
            charging: self.sys.1,
            network: Some(self.system_state.network),
            cursor,
            idle_seconds: {
                #[cfg(windows)] {Some(daozai::system::idle_seconds())}
                #[cfg(not(windows))] {None}
            },
            browser_at_top: {
                #[cfg(windows)] {if self.app.settings.browser_minimal {self.native.as_mut().and_then(|native|native.browser_at_top(&self.app.settings))}else{None}}
                #[cfg(not(windows))] {None}
            },
            media: self.media.lock().map(|m| m.clone()).unwrap_or_default(),
            volume: self.system_state.volume,
            bt: self.bt.clone(),
            bt_ready: self.bt_ready,
            screen,
            backdrop,
            glass_blur,
            opaque_surface: {
                #[cfg(windows)] {self.native.as_ref().is_some_and(|w|w.opaque_surface())}
                #[cfg(not(windows))] {false}
            },
        }
    }
}

impl eframe::App for Shell {
    fn raw_input_hook(&mut self,_ctx:&egui::Context,input:&mut egui::RawInput) {
        if input.viewport_id==egui::ViewportId::ROOT {self.app.filter_todo_input(input);}
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.resource_at.ready(Duration::from_secs(30)){
            let (count,bytes)={let textures=ctx.tex_manager();let textures=textures.read();(textures.num_allocated(),textures.allocated().map(|(_,info)|info.bytes_used()).sum())};
            daozai::resources::TEXTURES.store(count,std::sync::atomic::Ordering::Relaxed);
            daozai::resources::TEXTURE_BYTES.store(bytes,std::sync::atomic::Ordering::Relaxed);
            daozai::resources::GALLEYS.store(ctx.fonts(|fonts|fonts.num_galleys_in_cache()),std::sync::atomic::Ordering::Relaxed);
            self.resource_at=PollClock::now();
        }
        let guarded=daozai::resources::GUARD.tripped();
        if guarded && !self.guard_notified {
            // 资源保护熔断：停止音乐会话轮询（含封面、网易云日志）与歌词查询，保住用户电脑，重启后恢复。
            self.guard_notified=true;self.app.opts.fetch_lyrics=false;
            self.app.alert("Daozai paused music features","Unusual resource usage detected; music info and lyrics reading paused. Restart Daozai to resume; diagnostics are in the diagnostics folder of the data directory.",0,false);
        }
        self.media_enabled.store(self.app.settings.visible_pages[0] && !guarded,std::sync::atomic::Ordering::Relaxed);
        // 设置里改了岛体大小：下一帧起主窗口按新倍率栅格化，定位逻辑随像素比变化同步调整画布。
        #[cfg(windows)] eframe::set_root_viewport_scale(self.app.settings.island_scale());
        #[cfg(windows)]
        {
            // 用帧间隔限制刷新，绕过部分 Intel 驱动在垂直同步中的忙等待。
            let delay=Duration::from_secs_f64(1.0/60.0).saturating_sub(self.frame_at.elapsed());
            if !delay.is_zero(){std::thread::sleep(delay);}
            self.frame_at=Instant::now();
        }
        #[cfg(windows)]
        self.poll_tray();
        #[cfg(windows)]
        let foreground_changed=daozai::native::take_foreground_change();
        #[cfg(windows)]
        if foreground_changed {
            // 切到别的程序：收起，并在光标离开前不再自动展开
            self.app.foreground_changed();
        }
        #[cfg(windows)]
        let placement_key=(self.app.settings.display_id.clone(),self.app.settings.island_offset,self.app.settings.dock);
        #[cfg(windows)]
        if !self.placed || self.placement_scale!=ctx.pixels_per_point() || self.placement_key!=placement_key || self.placement_at.ready(Duration::from_secs(1)) {
            self.placement_at=PollClock::now();
            self.placement_scale=ctx.pixels_per_point();
            self.placement_key=placement_key;
            self.app.displays=daozai::display_layout::connected();
            let display=daozai::display_layout::selected(&self.app.displays,&self.app.settings.display_id);
            let offset=self.app.settings.island_offset_ratio();
            if display.is_some_and(|d|self.native.as_ref().is_some_and(|w|w.place_on_display(self.placement_scale,d.bounds,offset,self.app.settings.dock))) {
                if !self.placed {diagnostics::startup("Target screen positioned in physical coordinates, display requested");}
                self.placed=true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            } else if self.t0.elapsed()>=Duration::from_secs(2) {
                // Do not leave a hidden startup window forever when monitor discovery fails.
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
        }
        #[cfg(not(windows))]
        if !self.placed {
            if let Some(mon)=ctx.input(|i|i.viewport().monitor_size) {
                self.placed=true;self.win_x=((mon.x-app::WIN_W)/2.0).round();
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(self.win_x,0.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
        }
        #[cfg(windows)]
        {
            while let Ok(observation)=self.system_rx.try_recv(){
                let next=observation.input;
                let manual=next.is_some_and(|state|self.input_keys.as_ref().is_some_and(|keys|keys.allows(state.context)));
                if let Some(label)=self.input_state.observe(next,manual){
                    if let Some(keys)=&self.input_keys{keys.clear();}
                    if self.app.settings.system_hud{self.app.alert("Keyboard Language",label,5,false);}
                }
                let new=observation.system;
                // 切换输出设备时，系统会恢复该设备的音量；等待它稳定后再提示调节。
                if new.audio_endpoint!=self.system_state.audio_endpoint {self.volume_quiet=PollClock::now();}
                if self.system_ready && self.app.settings.system_hud {
                    for (title,body) in new.changes(&self.system_state) {
                        // 拖动音量条刚改过音量时，系统回声不再单独弹一条。
                        if title=="System Volume" && !self.volume_quiet.ready(Duration::from_millis(900)) { continue; }
                        self.app.alert(title,body,5,false);
                    }
                }
                self.system_state=new;
                self.system_ready=true;
                self.fullscreen=observation.fullscreen;
            }
            self.hidden=daozai::display_layout::hide_for_fullscreen(self.app.settings.hide_fullscreen,self.fullscreen,daozai::display_layout::selected(&self.app.displays,&self.app.settings.display_id).map(|d|d.bounds));
            // 毛玻璃：在岛体左右两侧和画布底边（都在窗口区域之外）取样，估计岛背后的明暗。
            if self.app.settings.skin.is_glass() && !self.hidden {
                let sampler=self.backdrop.get_or_insert_with(daozai::backdrop::Sampler::start);
                let ppp=ctx.pixels_per_point();
                if let Some(origin)=self.native.as_ref().and_then(|w|w.logical_origin(ppp)) {
                    let points=self.app.backdrop_points();
                    sampler.set_points(points.iter().map(|&(x,y)|(((origin.x+x)*ppp).round() as i32,((origin.y+y)*ppp).round() as i32)).collect());
                }
            } else {
                self.backdrop=None;
            }
            if self.app.settings.bluetooth_notifications {
                if self.bt_rx.is_none() {self.bt_rx=Some(daozai::bt::spawn());}
            } else {
                self.bt_rx=None;
                self.bt.clear();
                self.bt_ready=false;
            }
            if let Some(service)=&self.bt_rx {
                while let Ok(found)=service.rx.try_recv(){ self.bt=found; self.bt_ready=true; }
            }
            let wechat_enabled=self.app.settings.wechat_notifications && self.app.settings.notifications;
            if self.wechat.as_ref().is_some_and(|listener|!listener.registered()){self.wechat=None;}
            if !wechat_enabled {
                self.wechat=None;
                self.wechat_retry=None;
                // 微信开关开着却被总开关挡住时说清原因，否则用户只会看到「开了也不提醒」。
                if self.app.settings.wechat_notifications {
                    self.app.wechat_status="Not listening · reminders master switch is off".into();
                }else{self.app.wechat_status.clear();}
            }else if self.wechat.is_none() && self.wechat_retry.is_none_or(|at|at.elapsed()>=Duration::from_secs(10)){
                match daozai::wechat::Listener::new(ctx.clone()){
                    Ok(listener)=>{
                        self.wechat=Some(listener);
                        self.wechat_retry=None;
                        self.app.wechat_status="Listening · alerts when WeChat flashes in the taskbar".into();
                    }
                    Err(error)=>{
                        self.wechat_retry=Some(Instant::now());
                        self.app.wechat_status=format!("{error} · retrying in 10 s");
                    }
                }
            }
            let previous_pending=self.app.pending_messengers.len();
            for (pid,window) in self.wechat.as_ref().map(|listener|listener.take()).unwrap_or_default(){daozai::wechat::trace(format!("Island shows WeChat unread, account {pid}"));self.app.receive_wechat_reminder_from(pid,window);}
            if let Some(rx)=&self.incoming {
                while let Ok(event)=rx.try_recv(){match event {daozai::system::Incoming::Revoked=>{self.app.settings.read_notifications=false;self.app.settings.save();},daozai::system::Incoming::Status(s)=>self.app.listener_status=s,daozai::system::Incoming::Message(a,id,b)=>self.app.receive_notification(a,id,b)}}
            }
            if foreground_changed || self.app.pending_messengers.len()!=previous_pending {
                // 在微信 / QQ 里打开了某个账号的窗口：清掉这个账号的未读。
                let mut sources:Vec<app::Messenger>=self.app.pending_messengers.iter().map(|u|u.source).collect();sources.dedup();
                match daozai::wechat::foreground_client(&sources){
                    Some((source,pid))=>{
                        if self.app.pending_messengers.iter().any(|u|u.source==source){daozai::wechat::trace(format!("Island cleared {} account {pid} (foreground changed={foreground_changed})",source.name()));}
                        self.app.viewed_client(source,pid);
                    }
                    None=>if foreground_changed && !sources.is_empty(){daozai::wechat::trace(format!("Foreground changed but not a messenger main window: {}; island unread {:?}",daozai::wechat::describe_foreground(),self.app.pending_messengers.iter().map(|u|(u.source.name(),u.pid)).collect::<Vec<_>>()));},
                }
            }
            if let Some((unread,rx))=&self.tray_opening {
                let unread=*unread;
                match rx.try_recv(){
                    Ok(Ok(()))=>{self.tray_opening=None;self.opening_messenger=Some((unread,Instant::now()));ctx.request_repaint_after(Duration::from_secs(4));}
                    Ok(Err(error))=>{self.tray_opening=None;self.app.say(error);}
                    Err(std::sync::mpsc::TryRecvError::Disconnected)=>{self.tray_opening=None;self.app.say("Couldn't bring up WeChat. Please open it from the tray");}
                    Err(std::sync::mpsc::TryRecvError::Empty)=>{}
                }
            }
            if let Some((unread,started))=self.opening_messenger {
                if !self.app.pending_messengers.contains(&unread){self.opening_messenger=None;}
                else if started.elapsed()>=Duration::from_secs(4){
                    self.opening_messenger=None;
                    match daozai::wechat::foreground_client(&[unread.source]){
                        Some((source,pid)) if unread.pid==0||unread.pid==pid=>self.app.viewed_client(source,pid),
                        _=>self.app.say(format!("{} isn't in front yet, click the icon to retry",unread.source.name())),
                    }
                }
            }
        }
        if let Some(rx)=&self.seek_rx {
            if let Some(success)=match rx.try_recv(){Ok(success)=>Some(success),Err(std::sync::mpsc::TryRecvError::Disconnected)=>Some(false),Err(_)=>None}{
                self.seek_rx=None;
                if !success{self.app.say("Player didn't finish seeking. Open the app and retry");}
            }
        }
        let env = self.env(ctx);
        // v0.5.5 邮件：开关 / 间隔 / 账号清单变了就重新对齐收信线程，再把事件交给界面。
        let sig=(self.app.settings.mail_enabled,self.app.settings.mail_interval,self.app.mail_accounts().to_vec());
        if sig!=self.mail_sig{
            let accounts:&[daozai::mail::Account]=if sig.0{ &sig.2 }else{ &[] };
            self.mail.sync(accounts,sig.1);
            self.mail_sig=sig;
        }
        while let Some(ev)=self.mail.try_recv(){self.app.mail_event(ev);}
        // 日程连接或自动同步开关变化时下发给工作线程。
        // 录制/截图演示用的是假账号，别让真服务去连（会立刻失败、把状态改成红色）。
        #[cfg(feature="record")]
        let cal_demo=std::env::var("DAOZAI_CAL_DEMO").is_ok();
        #[cfg(not(feature="record"))]
        let cal_demo=false;
        let cal=if cal_demo{Vec::new()}else{self.app.calendar_configs()};
        let auto_sync=self.app.settings.calendar_auto_sync;
        if (&cal,auto_sync)!=(&self.cal_sig.0,self.cal_sig.1){
            self.cal.configure(cal.clone(),auto_sync);
            self.cal_sig=(cal,auto_sync);
        }
        while let Some(ev)=self.cal.try_recv(){self.app.calendar_event(ev);}
        #[cfg(feature="record")] let quota_demo=std::env::var("DAOZAI_QUOTA_DEMO").is_ok();
        #[cfg(not(feature="record"))] let quota_demo=false;
        if !quota_demo {
            let config=self.app.quota_config();let sig=(config.enabled,config.accounts.iter().cloned().map(|mut a|{a.alias.clear();a.pinned=false;a}).collect::<Vec<_>>());
            if self.quota_sig.as_ref()!=Some(&sig) || self.quota_at.ready(Duration::from_secs(180)){
                let accounts:Vec<_>=sig.1.iter().filter(|a|sig.0 && a.enabled).cloned().collect();
                if self.quota.refresh(accounts.clone()){self.app.quota_pending(&accounts);self.quota_sig=Some(sig);self.quota_at=PollClock::now();}
            }
            while let Some(ev)=self.quota.try_recv(self.app.quota_config()){self.app.quota_event(ev);}
        }
        let mut out = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::TRANSPARENT))
            .show(ctx, |ui| {
                // 左右停靠时画布是竖向的 SIDE_W × SIDE_H。
                let size=if self.app.settings.dock.is_side() {egui::vec2(daozai::side_dock::SIDE_W,daozai::side_dock::SIDE_H)} else {egui::vec2(app::WIN_W,app::WIN_H)};
                let win = egui::Rect::from_min_size(ui.max_rect().min,size);
                out = Some(self.app.frame(ui, win, &env));
            });
        let Some(out) = out else { return };

        #[cfg(windows)]
        if let Some(native) = &mut self.native {
            let r = |r: egui::Rect| (r.left(), r.top(), r.width(), r.height());
            // 保留主窗口的事件循环，避免全屏隐藏后设置窗口无法关闭或重新打开。
            let region=if self.hidden {(0.0,0.0,0.0,0.0)} else {r(out.region)};
            let hit=if self.hidden {(0.0,0.0,0.0,0.0)} else {r(out.hit)};
            native.update(region, hit, if self.hidden{None}else{out.notice_hit.map(r)},if self.hidden{None}else{out.file_drop_zone.map(r)},if self.hidden{None}else{out.glass.as_deref()},ctx.pixels_per_point(), out.editing, !self.hidden && out.expanded);
        }
        for cmd in out.cmds {
            match cmd {
                Cmd::Media(t) => {
                    let source=self.media.lock().map(|media|media.source.clone()).unwrap_or_default();
                    if self.media_control.is_none(){self.media_control=smtc::ControlWorker::start(ctx.clone());}
                    match self.media_control.as_ref().ok_or(()).and_then(|worker|worker.submit(t,source)){
                        Ok(Some(rx))=>self.seek_rx=Some(rx),Ok(None)=>{},Err(())=>self.app.say("Player is busy, please retry shortly"),
                    }
                }
                Cmd::ViewMessenger(unread)=>{
                    #[cfg(windows)] match daozai::wechat::open_unread(unread){
                        Ok(())=>{
                            match daozai::wechat::foreground_client(&[unread.source]){
                                Some((source,pid)) if unread.pid==0||unread.pid==pid=>self.app.viewed_client(source,pid),
                                _=>{self.opening_messenger=Some((unread,Instant::now()));ctx.request_repaint_after(Duration::from_secs(4));}
                            }
                        },
                        Err(_) if self.app.settings.wechat_tray_open && unread.source==app::Messenger::Wechat && self.tray_opening.is_none()=>{
                            match daozai::wechat::wechat_in_tray(unread){
                                Ok(())=>{
                                    // 缩在托盘：后台线程按下该账号的托盘按钮，不阻塞界面。
                                    let (tx,rx)=std::sync::mpsc::channel();let repaint=ctx.clone();
                                    std::thread::spawn(move||{let _=tx.send(daozai::wechat::open_wechat_from_tray(unread));repaint.request_repaint();});
                                    self.tray_opening=Some((unread,rx));
                                }
                                Err(error)=>self.app.say(error),
                            }
                        }
                        Err(error)=>self.app.say(error),
                    }
                }
                Cmd::Open(p) => {if let Err(e)=open_path(&p){self.app.say(e);}},
                Cmd::OpenUrl(url)=>{
                    if let Some(url)=daozai::mail::web_url(&url) {
                        if open_path(std::path::Path::new(&url)).is_err(){self.app.say("Can't open the default browser. Check Windows default apps settings");}
                    }
                },
                Cmd::CalendarImport=>{
                    #[cfg(windows)] match daozai::system::pick_files_for("Choose calendar file to import (.ics)",true){
                        Ok(files)=>if !files.is_empty(){
                            let read:Vec<(String,String)>=files.iter().map(|path|{
                                let name=path.file_name().map(|n|n.to_string_lossy().to_string()).unwrap_or_default();
                                let text=std::fs::read(path).ok().filter(|raw|raw.len()<=20*1024*1024).map(|raw|String::from_utf8_lossy(&raw).into_owned()).unwrap_or_default();
                                (name,text)
                            }).collect();
                            let note=self.app.import_ics(&read);self.app.say(note);
                        },
                        Err(e)=>self.app.say(format!("Can't choose file: {}",e.code())),
                    }
                },
                Cmd::AddFiles=>{
                    #[cfg(windows)] match daozai::system::pick_files(){Ok(files)=>if !files.is_empty(){self.app.on_drop(files);self.app.show_page(3);},Err(e)=>self.app.say(format!("Can't choose file: {}",e.code()))}
                },
                Cmd::DragFile(p)=>{
                    #[cfg(windows)] if let Err(e)=daozai::system::drag_file(&p){self.app.say(format!("Can't drag file out: {}",e.code()));}
                    ctx.input_mut(|i| i.pointer=Default::default());
                },
                Cmd::DisconnectNotifications=>{#[cfg(windows)]{self.incoming=None;self.app.settings.read_notifications=false;self.app.settings.save();self.app.listener_status="Disconnected, no longer reading Windows notifications".into();}},
                Cmd::Reveal(p)=>{let _=std::process::Command::new("explorer.exe").arg(format!("/select,{}",p.display())).spawn();},
                Cmd::QuotaChooseFile=>{
                    #[cfg(windows)] match daozai::system::pick_files_for("Choose the AI account's login JSON file",false){Ok(files)=>if let Some(path)=files.into_iter().next(){self.app.quota_set_source(path);},Err(_)=>self.app.say("Can't choose login file")}
                },
                Cmd::QuotaRefresh=>{
                    if self.quota_at.ready(Duration::from_secs(15)) {
                        let config=self.app.quota_config();let accounts:Vec<_>=config.accounts.iter().filter(|a|config.enabled && a.enabled).cloned().collect();
                        if self.quota.refresh(accounts.clone()){self.app.quota_pending(&accounts);self.quota_at=PollClock::now();}
                    }else{self.app.say("Please refresh again later");}
                },
                Cmd::QuotaConnected=>{self.quota_sig=None;},
                Cmd::ToggleAutostart=>{
                    #[cfg(windows)] {let want=!self.autostart;match daozai::tray::set_autostart(want){Ok(())=>self.autostart=want,Err(e)=>self.app.say(e)}}
                },
                Cmd::SetVolume(v)=>{
                    #[cfg(windows)] daozai::system::set_volume(v);
                    self.volume_quiet=PollClock::now();
                },
                Cmd::ToggleMute=>{
                    #[cfg(windows)] daozai::system::toggle_mute();
                    self.volume_quiet=PollClock::now();
                },
                Cmd::Copy(text)=>{ctx.copy_text(text);self.app.say("Copied");},
                Cmd::Notify(title,body)=>{
                    #[cfg(windows)] if let Some(tray)=&self.tray{tray.notify(&title,&body,self.app.settings.sound);}
                },
                cmd @ (Cmd::RequestNotifications|Cmd::RequestQqNotifications)=>{
                    #[cfg(windows)] {self.app.settings.qq_only=matches!(cmd,Cmd::RequestQqNotifications);self.app.settings.read_notifications=true;self.app.settings.save();self.incoming=None;match daozai::system::connect(ctx.clone(),self.app.settings.qq_only){Ok(rx)=>{self.incoming=Some(rx);self.app.listener_status="Requesting Windows notification permission…".into();},Err(e)=>self.app.listener_status=format!("Windows didn't allow reading notifications: {}. Requires the corresponding system permission or app identity.",e.code())}}
                },
                Cmd::MailSync=>{
                    let enabled=self.app.settings.mail_enabled;
                    let accounts=if enabled{self.app.mail_accounts().to_vec()}else{Vec::new()};
                    self.mail.sync(&accounts,self.app.settings.mail_interval);
                    self.mail_sig=(enabled,self.app.settings.mail_interval,self.app.mail_accounts().to_vec());
                },
                Cmd::MailTest(account,password)=>self.mail.test(account,password),
                Cmd::MailManage(request)=>self.mail.manage(request),
                Cmd::CalendarTest(cfg,password)=>self.cal.test(cfg,password),
                Cmd::CalendarSync=>self.cal.sync_now(),
                Cmd::CalendarMonth(month)=>self.cal.view_month(month),
                Cmd::CalendarForget=>{
                    if let Err(error)=daozai::calendar::forget_config(){self.app.say(&error);continue;}
                    if let Some(old)=self.app.forget_calendar(){daozai::calendar::secrets::delete(&old);}
                    let configs=self.app.calendar_configs();
                    self.cal.configure(configs.clone(),self.app.settings.calendar_auto_sync);
                    self.cal_sig=(configs,self.app.settings.calendar_auto_sync);
                },
                Cmd::Focus => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
            }
        }


        #[cfg(windows)] let autostart = self.autostart;
        #[cfg(not(windows))] let autostart = false;
        self.app.settings_ui(ctx, autostart);
        self.app.mail_reader_ui(ctx);
        #[cfg(windows)] let repaint_after=if self.hidden{Duration::from_secs(1)}else{out.repaint_after};
        #[cfg(not(windows))] let repaint_after=out.repaint_after;
        schedule_repaint(ctx,repaint_after);
    }
}

// ---------------- 系统信息 ----------------

fn schedule_repaint(ctx:&egui::Context,interval:Duration){
    // egui 会预减一帧；本程序自行调度刷新，补回后才能保持实际的等待间隔。
    let predicted=ctx.input(|i|Duration::from_secs_f32(i.predicted_dt));
    ctx.request_repaint_after(interval.saturating_add(predicted));
}

#[cfg(test)]
mod repaint_tests {
    use super::*;

    fn scheduled_delay(interval:Duration,immediate:bool)->Duration{
        let ctx=egui::Context::default();
        let mut delay=Duration::ZERO;
        for frame in 0..4 {
            let output=ctx.run(egui::RawInput{time:Some(frame as f64),predicted_dt:1.0/60.0,..Default::default()},|ctx|{
                if immediate{ctx.request_repaint();}
                schedule_repaint(ctx,interval);
            });
            delay=output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        delay
    }

    #[test]
    fn background_repaint_keeps_the_requested_frame_interval(){
        for fps in [60.0,30.0,20.0,10.0,1.0] {
            let interval=Duration::from_secs_f64(1.0/fps);
            let actual=scheduled_delay(interval,false);
            assert!(actual.abs_diff(interval)<Duration::from_micros(1),"目标 {fps} 帧不应被提前唤醒：{actual:?}");
        }
    }

    #[test]
    fn interaction_and_service_events_still_repaint_immediately(){
        assert_eq!(scheduled_delay(Duration::from_secs(1),true),Duration::ZERO);
    }
}

#[cfg(windows)]
fn local_now() -> (String, String, String) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let st = unsafe { GetLocalTime() };
    let wd = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    (
        format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond),
        format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay),
        format!("{}/{} {}", st.wMonth, st.wDay, wd[(st.wDayOfWeek as usize).min(6)]),
    )
}

#[cfg(not(windows))]
fn local_now() -> (String, String, String) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    (format!("{h:02}:{m:02}:{s:02}"), "1970-01-01".into(), "1/1 Thu".into())
}

#[cfg(windows)]
fn read_power() -> (Option<u8>, bool) {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut st = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut st) }.is_ok() {
        let pct = if st.BatteryLifePercent == 255 { None } else { Some(st.BatteryLifePercent) };
        (pct, st.ACLineStatus == 1)
    } else {
        (None, false)
    }
}

#[cfg(not(windows))]
fn read_power() -> (Option<u8>, bool) {
    (None, true)
}

#[cfg(windows)]
fn cursor_logical(ppp: f32) -> Option<(f32, f32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) }.ok()?;
    Some((p.x as f32 / ppp, p.y as f32 / ppp))
}

#[cfg(not(windows))]
fn cursor_logical(_ppp: f32) -> Option<(f32, f32)> {
    None
}

#[cfg(windows)]
fn open_path(p: &std::path::Path)->Result<(),String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{ShellExecuteExW,SHOpenWithDialog,SHELLEXECUTEINFOW,OPENASINFO,SEE_MASK_FLAG_NO_UI,OAIF_EXEC};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let mut wide: Vec<u16> = p.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut info=SHELLEXECUTEINFOW{cbSize:std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,fMask:SEE_MASK_FLAG_NO_UI,lpVerb:windows::core::w!("open"),lpFile:PCWSTR(wide.as_ptr()),nShow:SW_SHOWNORMAL.0,..Default::default()};
    open_path_result(p,unsafe{ShellExecuteExW(&mut info)},||unsafe{
        SHOpenWithDialog(None,&OPENASINFO{pcszFile:PCWSTR(wide.as_ptr()),oaifInFlags:OAIF_EXEC,..Default::default()})
    })
}

#[cfg(windows)]
fn open_path_result(p:&std::path::Path,result:windows::core::Result<()>,choose:impl FnOnce()->windows::core::Result<()>)->Result<(),String>{
    use windows::Win32::Foundation::{ERROR_CANCELLED,ERROR_NO_ASSOCIATION};
    let result=match result{
        Err(error) if error.code()==windows::core::HRESULT::from_win32(ERROR_NO_ASSOCIATION.0)&&p.is_file()=>choose(),
        other=>other,
    };
    match result{
        Err(error) if error.code()==windows::core::HRESULT::from_win32(ERROR_CANCELLED.0)=>Ok(()),
        other=>other.map_err(|_|"Can't open. Check the file location and default app".into()),
    }
}

#[cfg(not(windows))]
fn open_path(_p: &std::path::Path)->Result<(),String> {Ok(())}

#[cfg(all(test,windows))]
mod open_path_tests {
    use super::*;
    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED,ERROR_CANCELLED,ERROR_FILE_NOT_FOUND,ERROR_NO_ASSOCIATION};
    fn error(code:windows::Win32::Foundation::WIN32_ERROR)->windows::core::Error{windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(code.0))}

    #[test]
    fn unassociated_files_offer_open_with_and_cancellation_is_normal(){
        let path=std::env::temp_dir().join(format!("岛仔 无关联-{}.daozai-open-test",std::process::id()));
        std::fs::write(&path,b"fixture").unwrap();
        for cancel in [false,true]{
            let mut chosen=false;
            let result=open_path_result(&path,Err(error(ERROR_NO_ASSOCIATION)),||{chosen=true;if cancel{Err(error(ERROR_CANCELLED))}else{Ok(())}});
            std::fs::remove_file(&path).unwrap();
            assert!(chosen,"文件没有默认关联时应提供系统打开方式");
            assert_eq!(result,Ok(()),"用户取消选择不应显示打开失败");
            if !cancel{std::fs::write(&path,b"fixture").unwrap();}
        }
    }

    #[test]
    fn default_apps_urls_and_other_errors_never_open_the_file_picker(){
        let file=std::env::temp_dir().join(format!("daozai-open-errors-{}",std::process::id()));
        std::fs::write(&file,b"fixture").unwrap();
        for result in [Ok(()),Err(error(ERROR_ACCESS_DENIED)),Err(error(ERROR_FILE_NOT_FOUND))]{
            let success=result.is_ok();
            let actual=open_path_result(&file,result,||panic!("已有默认程序或其他打开错误不应弹出选择器"));
            assert_eq!(actual.is_ok(),success);
        }
        assert!(open_path_result(&file,Err(error(ERROR_NO_ASSOCIATION)),||Err(error(ERROR_ACCESS_DENIED))).is_err(),"选择器失败应保留错误提示");
        std::fs::remove_file(file).unwrap();
        for (path,result,success) in [
            ("https://example.com",Ok(()),true),
            ("https://example.com",Err(error(ERROR_NO_ASSOCIATION)),false),
            ("missing.daozai-open-test",Err(error(ERROR_NO_ASSOCIATION)),false),
            ("missing.daozai-open-test",Err(error(ERROR_FILE_NOT_FOUND)),false),
            ("blocked.daozai-open-test",Err(error(ERROR_ACCESS_DENIED)),false),
        ]{
            let actual=open_path_result(std::path::Path::new(path),result,||panic!("非文件关联问题不能弹出打开方式"));
            assert_eq!(actual.is_ok(),success);
        }
    }
}

fn main() -> std::process::ExitCode {
    diagnostics::install();
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            diagnostics::report("Daozai failed to start or run.", &error.to_string());
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> eframe::Result<()> {
    // 单实例（与原版同名互斥体）
    #[cfg(windows)]
    if !daozai::tray::single_instance() {
        if !daozai::tray::activate_existing(std::env::args().any(|arg|arg=="--calendar")) {
            diagnostics::report("Daozai is already running but can't open its window right now.\nPlease retry later; if it still won't open, end daozai.exe in Task Manager's 'Details' tab and retry.", "The existing instance may still be starting or not responding. Please retry later; if it still won't open, end daozai.exe in Task Manager's 'Details' tab, then restart.");
        }
        return Ok(());
    }
    #[cfg(windows)]
    std::thread::spawn(daozai::record_install_size);
    diagnostics::startup("Initialize graphics window");
    let viewport = egui::ViewportBuilder::default()
        .with_title("岛仔").with_icon(daozai::theme::app_icon())
        .with_inner_size([app::CANVAS_W, app::WIN_H])
        .with_decorations(false)
        .with_transparent(true)
        .with_active(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_always_on_top()
        .with_visible(false);
    let opts = eframe::NativeOptions { viewport, vsync: !cfg!(windows), ..Default::default() };
    graphics::run(opts,|opts|eframe::run_native("岛仔", opts, Box::new(|cc| Ok(Box::new(Shell::new(cc))))))
}
