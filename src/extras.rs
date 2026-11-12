//! 设置、通知队列与岛仔表情。
use crate::settings_theme::color as settings_color;
use crate::app::{App, Cmd, Phase, HUD_TRACK_W, HUD_TRACK_X, HUD_W, PAGE_NAMES};
use crate::paint::{Card, H};
use crate::theme::{self, size, W};
use eframe::egui::{self, Color32, Rect};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AlertSource { Internal, Wechat, External(String) }

#[derive(Clone, Debug)]
pub(crate) struct Alert {
    pub title: String,
    pub source: AlertSource,
    pub body: String,
    pub page: usize,
    pub at: f64,
    pub duration: f64,
    pub bluetooth: Option<(crate::bt::DeviceKind, bool)>,
    pub meeting: Option<(crate::mail::Meeting, String)>,
}
impl Alert {
    pub(crate) fn messenger(&self)->Option<crate::app::Messenger>{
        match &self.source {
            AlertSource::Wechat=>Some(crate::app::Messenger::Wechat),
            AlertSource::External(id) if id=="QQ"=>Some(crate::app::Messenger::Qq),
            _=>None,
        }
    }
    pub(crate) fn is_hud(&self)->bool {self.source==AlertSource::Internal && matches!(self.title.as_str(),"System Volume"|"Caps Lock"|"Keyboard Input Language"|"Network"|"Power Connected"|"On Battery Power"|"Bluetooth Device")}
}
fn bluetooth_notice_content(body:&str)->(&str,&str,Option<u8>){
    if let Some(name)=body.strip_suffix(" · Disconnected") {return (name,"Disconnected",None);}
    if body=="Disconnected" {return ("Bluetooth Device","Disconnected",None);}
    if let Some((name,level))=body.rsplit_once(" · Battery ") {
        if let Some(level)=level.strip_suffix('%').and_then(|s|s.parse::<u8>().ok()).filter(|v|*v<=100){
            return (name,"Connected",Some(level));
        }
    }
    (body,"Connected",None)
}

pub(crate) fn device_icon(c: &Card, kind: crate::bt::DeviceKind, x: f32, y: f32, size: f32) {
    let icon=match kind {
        crate::bt::DeviceKind::Bone | crate::bt::DeviceKind::Earbuds | crate::bt::DeviceKind::Headphones=>egui::include_image!("../assets/device-icons/headphones-round-bold.svg"),
        crate::bt::DeviceKind::Speaker=>egui::include_image!("../assets/device-icons/speaker-bold.svg"),
        crate::bt::DeviceKind::Keyboard=>egui::include_image!("../assets/device-icons/keyboard-bold.svg"),
        crate::bt::DeviceKind::Mouse=>egui::include_image!("../assets/device-icons/mouse-bold.svg"),
        crate::bt::DeviceKind::Gamepad=>egui::include_image!("../assets/device-icons/gamepad-bold.svg"),
        crate::bt::DeviceKind::Phone=>egui::include_image!("../assets/device-icons/smartphone-bold.svg"),
        crate::bt::DeviceKind::Other=>egui::include_image!("../assets/device-icons/bluetooth-linear.svg"),
    };
    let rect=c.rect(x,y,size,size);
    let egui::ImageSource::Bytes{uri,bytes}=icon else{return};
    let pixels=(rect.width()*c.ui.ctx().pixels_per_point()).ceil() as u32;
    let icon=egui::Image::from_bytes(format!("{uri}/{pixels}.svg"),bytes);
    if let Ok(egui::load::TexturePoll::Ready { texture })=icon.fit_to_exact_size(rect.size()).load_for_size(c.ui.ctx(),rect.size()) {
        c.painter.image(texture.id,rect,Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);
    }
}

pub(crate) use crate::pet::PetState;

pub(crate) fn notice_badge(c:&Card,x:f32,y:f32,a:&Alert){
    if a.source==AlertSource::Internal && a.title=="Bluetooth Device"{
        c.circle(x,y,11.0,if a.body.contains("Disconnected"){Color32::from_gray(95)}else{theme::BLUE});
        device_icon(c,a.bluetooth.map_or(crate::bt::DeviceKind::Other,|v|v.0),x-7.5,y-7.5,15.0);
    }else{notice_icon(&c.scaled(x,y,0.85),x,y,a);}
}
pub(crate) fn messenger_icon(c:&Card,x:f32,y:f32,source:crate::app::Messenger){

    let ink=Color32::from_gray(24);
    if source==crate::app::Messenger::Wechat {
        let back=Color32::from_rgb(70,203,123);
        let front=Color32::from_rgb(145,235,165);
        c.fill(x-10.,y-7.,14.,11.,5.5,back);
        c.poly(&[(x-8.,y+1.),(x-8.,y+6.),(x-3.,y+2.)],back);
        for dx in [-6.,-1.]{c.circle(x+dx,y-3.,0.9,ink);}
        c.fill(x-2.,y-2.,13.,10.,5.,front);
        c.poly(&[(x+5.,y+6.),(x+9.,y+10.),(x+8.,y+5.)],front);
        for dx in [2.,7.]{c.circle(x+dx,y+1.,0.8,ink);}
    }else{
        let rect=c.rect(x-9.5,y-9.5,19.0,19.0);
        let icon=egui::Image::new(egui::include_image!("../assets/qq-client.png"));
        if let Ok(egui::load::TexturePoll::Ready{texture})=icon.load_for_size(c.ui.ctx(),rect.size()){
            c.painter.image(texture.id,rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);
        }
    }
}
fn notice_icon(c:&Card,x:f32,y:f32,a:&Alert) {
    if a.source!=AlertSource::Internal{crate::paint::bell(c,x,y,theme::WHITE,0.0);return;}
    match a.title.as_str() {
        "System Volume"=>{
            c.poly(&[(x-9.0,y-3.0),(x-5.0,y-3.0),(x,y-7.0),(x,y+7.0),(x-5.0,y+3.0),(x-9.0,y+3.0)],theme::WHITE);
            c.arc(x,y,5.0,-0.8,0.8,1.5,theme::WHITE);c.arc(x,y,8.0,-0.7,0.7,1.3,theme::WHITE);
        },
        "Caps Lock"=>{
            let ink=if a.body=="On"{theme::GREEN}else{theme::GRAY};
            c.poly(&[(x-8.0,y-1.0),(x,y-9.0),(x+8.0,y-1.0)],ink);
            c.fill(x-3.0,y-1.5,6.0,6.5,0.5,ink);
            c.fill(x-4.0,y+7.0,8.0,2.0,0.8,ink);
        },
        "Keyboard Input Language"=>{
            c.stroke(x-10.0,y-8.5,20.0,17.0,4.0,1.2,theme::GRAY);
            let label=match a.body.as_str(){"Chinese"=>"中","English"=>"EN",_=>"Aa"};
            c.centered_text(x,y,label,if label=="EN"{9.0}else{11.0},W::Semibold,theme::WHITE,H::Center);
        },
        "Network"=>{
            let connected=a.body.contains("Connected");
            let ink=if connected{theme::GREEN}else{theme::GRAY};
            if a.body.starts_with("Wi-Fi") {
                for r in [4.0,8.0,12.0]{c.arc(x,y+6.0,r,-2.35,-0.79,1.7,ink);}
                c.circle(x,y+6.0,1.6,ink);
            }else if a.body.starts_with("Ethernet") {
                c.stroke(x-8.0,y-8.0,16.0,14.0,2.5,1.6,ink);
                for dx in [-4.0,-1.3,1.3,4.0]{c.seg((x+dx,y-5.0),(x+dx,y-1.0),1.2,ink);}
                c.path(&[(x-4.0,y+6.0),(x-4.0,y+9.0),(x+4.0,y+9.0),(x+4.0,y+6.0)],1.6,ink);
            }else{
                c.ring(x,y,8.0,1.4,ink);c.seg((x-8.0,y),(x+8.0,y),1.0,ink);
                c.stroke(x-3.5,y-8.0,7.0,16.0,3.5,1.0,ink);
            }
            if !connected{c.seg((x-8.0,y-8.0),(x+9.0,y+9.0),2.0,theme::WHITE);}
        },
        "Power Connected"|"On Battery Power"|"Low Battery"=>{
            let low=a.title=="Low Battery";let ink=if low{theme::RED_TOP}else if a.title=="Power Connected"{theme::GREEN}else{theme::WHITE};
            c.stroke(x-11.0,y-6.0,20.0,12.0,3.0,1.5,ink);c.fill(x+10.0,y-2.5,2.0,5.0,0.8,ink);
            if low{c.fill(x-8.0,y-3.5,3.5,7.0,1.0,ink);}
            else if a.title=="Power Connected"{
                c.poly(&[(x,y-4.5),(x-4.0,y+0.8),(x+0.5,y+0.8)],ink);
                c.poly(&[(x+0.5,y-0.8),(x+4.0,y-0.8),(x-1.0,y+4.5)],ink);
            }else{c.fill(x-8.0,y-3.5,12.0,7.0,1.0,ink);}
        },
        "Time to Drink Water"=>{
            c.poly(&[(x-7.0,y-1.0),(x+5.0,y-1.0),(x+3.5,y+8.0),(x-5.5,y+8.0)],theme::BLUE);
            c.path(&[(x-9.0,y-10.0),(x-6.5,y+9.0),(x+4.5,y+9.0),(x+7.0,y-10.0)],1.6,theme::CYAN);
            c.seg((x-9.0,y-10.0),(x+7.0,y-10.0),1.6,theme::CYAN);
            c.path(&[(x+6.5,y-6.0),(x+10.5,y-6.0),(x+11.5,y-3.0),(x+10.0,y+2.0),(x+5.5,y+3.0)],1.6,theme::CYAN);
            c.seg((x-5.0,y-6.5),(x-4.2,y-2.5),1.0,Color32::from_white_alpha(170));
        },
        "To-do Due"=>crate::paint::checklist(c,x,y,23.0,Color32::from_rgb(190,180,255),Color32::from_rgb(43,35,66)),
        "Meeting Reminder"=>{
            let label=a.meeting.as_ref().map(|(_,source)|source.as_str()).unwrap_or("");
            let source=if label=="Local Calendar"{crate::calendar_ui::Source::Local}else if label.starts_with("Mail"){crate::calendar_ui::Source::Mail("")}
                else{crate::calendar_ui::Source::Calendar(crate::calendar::Provider::ALL.into_iter().find(|provider|provider.label()==label))};
            crate::calendar_ui::source_icon(c,source,x-14.0,y-14.0,28.0);
        },
        "Focus Timer"=>crate::paint::stopwatch(&c.scaled(x,y,0.65),x,y,8.0,0.0,theme::RED_TOP),
        _=>crate::paint::bell(&c.scaled(x,y,1.7),x,y,theme::GRAY,0.0),
    }
}

impl App {
    pub(crate) fn phase_secs(&self, phase: Phase) -> f32 {
        (match phase {
            Phase::Focus => self.settings.focus_minutes,
            Phase::Rest => self.settings.rest_minutes,
            Phase::Stopwatch => 60,
        }) as f32
            * 60.0
    }
    pub(crate) fn water_hours(&self, hour: u32) -> bool {
        let (a, b) = (self.settings.water_start, self.settings.water_end);
        if a == b {
            true
        } else if a < b {
            hour >= a && hour < b
        } else {
            hour >= a || hour < b
        }
    }
    pub(crate) fn pet_state(&self) -> PetState {
        if let Some((state, _)) = self.pet.reaction {
            state
        } else if self.pet.waking_until > self.t {
            PetState::Waking
        } else if self.pomo.running && self.pomo.phase.is_focus_like() {
            PetState::Focused
        } else if self.pet.worried {
            PetState::Worried
        } else if self.media.playing && self.media.is_music() {
            if self.notice.is_some() || self.hud_notice.is_some() {PetState::Awake}else{PetState::Music}
        } else if self.pet.hovered {
            PetState::Curious
        } else if self.idle_for() >= 180.0 {
            PetState::Sleeping
        } else if self.idle_for() >= self.opts.pet_idle {
            PetState::Dozing
        } else {
            PetState::Awake
        }
    }
    pub fn alert(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        page: usize,
        system: bool,
    ) {
        let title = title.into();
        let transient = matches!(
            title.as_str(),
            "System Volume" | "Caps Lock" | "Keyboard Input Language" | "Network" | "Power Connected" | "On Battery Power" | "Bluetooth Device"
        );
        let a = Alert {
            source: AlertSource::Internal,
            title,
            body: body.into(),
            page,
            at: self.t,
            duration: if transient { 2.5 } else { 8.0 },
            bluetooth: None,
            meeting: None,
        };
        self.enqueue_alert(a, system);
    }
    pub(crate) fn meeting_alert(&mut self, meeting: crate::mail::Meeting, source: String, page: usize) {
        self.enqueue_alert(Alert {
            source: AlertSource::Internal,
            title: "Meeting Reminder".into(), body: format!("{} · {} · {}", meeting.title, meeting.at, source),
            page, at: self.t, duration: 12.0, bluetooth: None, meeting: Some((meeting, source)),
        }, true);
    }
    pub(crate) fn bluetooth_alert(&mut self, device: &crate::bt::Device, connected: bool) {
        if !self.settings.bluetooth_notifications { return; }
        if connected && matches!(device.kind,crate::bt::DeviceKind::Bone|crate::bt::DeviceKind::Earbuds|crate::bt::DeviceKind::Headphones|crate::bt::DeviceKind::Speaker) {
            self.alerts.retain(|a|a.title!="System Volume");
            if self.notice.as_ref().is_some_and(|a|a.title=="System Volume") {self.notice=None;}
        }
        let a = Alert {
            source: AlertSource::Internal,
            title: "Bluetooth Device".into(),
            body: if connected { device.summary() } else { format!("{} · Disconnected", device.name) },
            page: 5, at: self.t, duration: 2.5,
            bluetooth: Some((device.kind, connected)),
            meeting: None,
        };
        self.enqueue_alert(a, false);
    }
    fn enqueue_alert(&mut self, a: Alert, system: bool) {
        // 通知仅保存在本次运行的内存中。
        if a.source==AlertSource::Internal {self.history.push_front(a.clone());}
        self.history.truncate(50);
        if self.settings.notifications {
            if system && a.source==AlertSource::Internal {
                self.pending
                    .push(Cmd::Notify(a.title.clone(), a.body.clone()));
            }
            let same_event=|old:&Alert| match (&a.meeting,&old.meeting) {
                (Some((next,_)),Some((previous,_)))=>crate::calendar::dedupe_key(next)==crate::calendar::dedupe_key(previous),
                _=>old.source==a.source && old.title==a.title && (a.is_hud() || old.body==a.body),
            };
            if a.source!=AlertSource::Internal || a.title != "Bluetooth Device" { self.alerts.retain(|old| !same_event(old)); }
            if (a.source!=AlertSource::Internal || a.title != "Bluetooth Device") && self.notice.as_ref().is_some_and(same_event) {
                self.notice = Some(a);
                return;
            }
            self.alerts.push_back(a);
            while self.alerts.len() > 8 {
                self.alerts.pop_front();
            }
        }
    }
    /// 该来源的所有账号都已查看。
    pub fn viewed_messenger(&mut self,source:crate::app::Messenger){
        self.pending_messengers.retain(|u|u.source!=source);
        self.clear_messenger_notices(source);
    }
    /// 某个账号（进程）的窗口被打开：只清它自己；分不出账号的条目（pid 0）一并清掉。
    pub fn viewed_client(&mut self,source:crate::app::Messenger,pid:u32){
        self.pending_messengers.retain(|u|u.source!=source || pid!=0 && u.pid!=0 && u.pid!=pid);
        if !self.pending_messengers.iter().any(|u|u.source==source){self.clear_messenger_notices(source);}
    }
    fn clear_messenger_notices(&mut self,source:crate::app::Messenger){
        self.alerts.retain(|a|a.messenger()!=Some(source));
        if self.notice.as_ref().is_some_and(|a|a.messenger()==Some(source)){self.notice=None;self.notification_bell_until=0.0;}
    }
    fn mark_messenger(&mut self,source:crate::app::Messenger,pid:u32,window:isize){
        if let Some(u)=self.pending_messengers.iter_mut().find(|u|u.source==source && u.pid==pid){
            if window!=0 {u.window=window;}
            return;
        }
        self.pending_messengers.push(crate::app::Unread{source,pid,window});
    }
    /// 横幅「查看」：跳到该来源最近一条未读的窗口。
    pub(crate) fn latest_unread(&self,source:crate::app::Messenger)->crate::app::Unread{
        self.pending_messengers.iter().rev().find(|u|u.source==source).copied().unwrap_or(crate::app::Unread::of(source))
    }
    pub fn receive_wechat_reminder(&mut self){self.receive_wechat_reminder_from(0,0);}
    /// 微信窗口闪动：`pid` 是该账号的进程，`window` 是闪动的窗口。
    pub fn receive_wechat_reminder_from(&mut self,pid:u32,window:isize){
        if !self.settings.wechat_notifications || !self.settings.notifications{return;}
        self.mark_messenger(crate::app::Messenger::Wechat,pid,window);
        self.notification_bell_at=self.t;
        self.notification_bell_until=self.t+4.5;
        self.enqueue_alert(Alert{source:AlertSource::Wechat,title:"WeChat".into(),body:"New message · Check WeChat".into(),page:usize::MAX,at:self.t,duration:4.5,bluetooth:None,meeting:None},false);
    }
    pub fn receive_notification(&mut self, app:String, id:String, body:String){
        if !self.settings.read_notifications || id.is_empty() || (self.settings.qq_only && id!="QQ"){return;}
        // 岛仔自己发出的系统提醒会被通知中心回传；已在历史里的同一内容不再入队，避免重复显示。
        if self.history.iter().any(|a|format!("{} · {}",a.title,a.body)==body){return;}
        if !self.notification_apps.observe(&id,&app) || !self.settings.notifications{return;}
        if id=="QQ"{self.mark_messenger(crate::app::Messenger::Qq,0,0);}
        self.notification_bell_at=self.t;
        self.notification_bell_until=self.t+4.5;
        let title=app.chars().filter(|c|!c.is_control()).take(80).collect();
        let body=if self.settings.notification_preview {
            let mut text=body.chars().filter(|c|!c.is_control()).take(300).collect::<String>();
            if let Some(code)=crate::otp::extract("",&text,&[]) {text=crate::mail_privacy::mask(&text,&code.text);}
            text
        }else{"New notification · Content hidden".into()};
        self.enqueue_alert(Alert{source:AlertSource::External(id),title,body,page:usize::MAX,at:self.t,duration:4.5,bluetooth:None,meeting:None},false);
    }
    pub(crate) fn tick_alerts(&mut self) {
        self.pending_messengers.retain(|u|self.settings.notifications && match u.source {
            crate::app::Messenger::Wechat=>self.settings.wechat_notifications,
            crate::app::Messenger::Qq=>self.settings.read_notifications && self.notification_apps.items.iter().any(|(key,_,on)|key=="QQ" && *on),
        });
        let allowed=|a:&Alert|match &a.source {
            AlertSource::Internal=>true,
            AlertSource::Wechat=>self.settings.wechat_notifications && self.settings.notifications,
            AlertSource::External(id)=>self.settings.read_notifications && (!self.settings.qq_only || id=="QQ") && self.settings.notifications && self.notification_apps.items.iter().any(|(key,_,on)|key==id && *on),
        };
        self.alerts.retain(&allowed);
        if self.notice.as_ref().is_some_and(|a|!allowed(a)){self.notice=None;}
        if self.hud_notice.as_ref().is_some_and(|a|!allowed(a)){self.hud_notice=None;}
        if !self.settings.notification_preview {
            for a in self.alerts.iter_mut().chain(self.notice.iter_mut()).chain(self.hud_notice.iter_mut()) {
                if matches!(&a.source,AlertSource::External(_)){a.body="New notification · Content hidden".into();}
            }
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|a| self.t - a.at > a.duration)
        {
            self.notice = None;
        }
        if self.notice.is_none() {
            if let Some(mut a) = self.alerts.pop_front() {
                a.at = self.t;
                self.notice = Some(a);
            }
        }
    }
    pub(crate) fn draw_notice(&mut self, c:&Card,panel:Rect,a:&Alert,_cmds:&mut Vec<Cmd>){
        let nc=c.clipped(0.0,0.0,panel.width(),panel.height());
        if let Some(source)=a.messenger(){
            messenger_icon(&nc,22.0,22.0,source);
            nc.circle(31.0,14.0,2.6,Color32::from_rgb(255,72,72));
            nc.centered_text(44.0,14.0,&a.title,11.0,W::Semibold,theme::WHITE,H::Left);
            nc.centered_text(44.0,30.0,&nc.fit(&a.body,10.5,W::Regular,154.0),10.5,W::Regular,theme::GRAY,H::Left);
            if notice_button(&nc,206.0,12.0,52.0,"View","messenger-view"){_cmds.push(Cmd::ViewMessenger(self.latest_unread(source)));}
            if notice_button(&nc,265.0,12.0,52.0,"Close","messenger-dismiss"){self.notice=None;self.notification_bell_until=0.0;}
            return;
        }
        if a.source!=AlertSource::Internal {
            crate::paint::bell(&nc,310.0,22.0,theme::GRAY,crate::paint::bell_swing(self.t-a.at));
            nc.circle(314.0,17.5,2.0,Color32::from_rgb(255,72,72));
            nc.centered_text(16.0,14.0,&nc.fit(&a.title,11.0,W::Semibold,198.0),11.0,W::Semibold,theme::WHITE,H::Left);
            nc.centered_text(16.0,30.0,&nc.fit(&a.body,10.5,W::Regular,198.0),10.5,W::Regular,theme::GRAY,H::Left);
            if notice_button(&nc,226.0,12.0,60.0,"Close","external-dismiss"){
                self.notice=None;
                self.notification_bell_until=0.0;
            }
            return;
        }
        if let Some((meeting,_))=&a.meeting {
            let secondary=Color32::from_gray(174);
            notice_icon(&nc.scaled(34.0,46.0,36.0/28.0),34.0,46.0,a);
            let now=chrono::Local::now();
            let start=chrono::DateTime::from_timestamp(meeting.start,0).unwrap().with_timezone(&chrono::Local);
            let left=meeting.start-now.timestamp();
            let countdown=if left>0 {format!("in {} min",(left+59)/60)}else{"Starting now".into()};
            let countdown_width=nc.text_w(&countdown,10.5,W::Semibold)+16.0;
            let countdown_x=344.0-countdown_width;
            nc.centered_text(64.0,33.0,&start.format("%H:%M").to_string(),30.0,W::Semibold,theme::WHITE,H::Left);
            nc.fill(countdown_x,23.0,countdown_width,20.0,10.0,Color32::from_gray(28));
            nc.centered_text(countdown_x+countdown_width/2.0,33.0,&countdown,10.5,W::Semibold,theme::WHITE,H::Center);
            let overflow=(nc.text_w(&meeting.title,size::TITLE,W::Semibold)-280.0).max(0.0);
            let offset=(((self.t-a.at-1.2).max(0.0) as f32)*24.0).min(overflow);
            nc.clipped(64.0,52.0,280.0,24.0).centered_text(64.0-offset,64.0,&meeting.title,size::TITLE,W::Semibold,theme::WHITE,H::Left);
            nc.seg((16.0,84.0),(344.0,84.0),0.6,Color32::from_gray(36));
            let date=format!("{}{}",if start.date_naive()==now.date_naive(){"Today · "}else{""},start.format("%b %-d"));
            nc.centered_text(16.0,110.0,&date,11.0,W::Regular,secondary,H::Left);
            let (dismiss,dc)=crate::paint::capsule(&nc,188.0,98.0,72.0,24.0,"meeting-dismiss",Color32::from_gray(32),Color32::from_gray(48));
            dc.centered_text(224.0,110.0,"Close",10.5,W::Semibold,theme::WHITE,H::Center);
            if dismiss.on_hover_text("Dismiss this reminder; the event is kept").clicked(){
                self.notice=None;
                return;
            }
            let (button,bc)=crate::paint::capsule(&nc,272.0,98.0,72.0,24.0,"meeting-action",Color32::from_gray(42),Color32::from_gray(56));
            bc.centered_text(308.0,110.0,"View",10.5,W::Semibold,theme::WHITE,H::Center);
            if button.clicked() {
                self.notice=None;
                self.open_calendar(Some(meeting.clone()));
            }
            return;
        }
        if a.is_hud(){
            if a.title=="Bluetooth Device" {
                let (name,status,battery)=bluetooth_notice_content(&a.body);
                nc.circle(26.0,24.0,14.0,if status=="Connected"{Color32::from_rgb(10,132,255)}else{Color32::from_gray(95)});
                let kind=a.bluetooth.map(|(kind,_)|kind).unwrap_or(crate::bt::DeviceKind::Other);
                device_icon(&nc,kind,17.0,15.0,18.0);
                let right=HUD_W-14.0;
                let name_end=if let Some(level)=battery {
                    let pct=format!("{level}%");
                    let pct_width=nc.text_w(&pct,10.5,W::Semibold);
                    let x=right-pct_width-22.0;
                    nc.stroke(x,20.0,15.0,8.0,2.0,1.0,Color32::from_gray(175));
                    nc.fill(x+15.5,22.5,2.0,3.0,0.8,Color32::from_gray(175));
                    let fill=11.0*level as f32/100.0;
                    if fill>0.0 {nc.fill(x+2.0,22.0,fill,4.0,1.0,if level<=20{theme::RED}else{theme::GREEN});}
                    nc.centered_text(right,24.0,&pct,10.5,W::Semibold,theme::WHITE,H::Right);
                    x-8.0
                } else {right};
                nc.centered_text(49.0,16.0,&nc.fit(name,11.0,W::Semibold,(name_end-49.0).max(20.0)),11.0,W::Semibold,theme::WHITE,H::Left);
                nc.centered_text(49.0,32.0,status,10.5,W::Regular,theme::GRAY,H::Left);
                return;
            }
            match a.title.as_str(){
                "System Volume"=>{
                    nc.poly(&[(17.0,16.0),(21.0,16.0),(26.0,12.0),(26.0,26.0),(21.0,22.0),(17.0,22.0)],theme::WHITE);
                    if self.volume_ui.is_some_and(|(_,muted)|muted){nc.seg((29.0,16.0),(34.0,22.0),1.4,theme::WHITE);nc.seg((34.0,16.0),(29.0,22.0),1.4,theme::WHITE);}
                    else{nc.arc(26.0,19.0,5.0,-0.8,0.8,1.5,theme::WHITE);nc.arc(26.0,19.0,8.0,-0.7,0.7,1.3,theme::WHITE);}
                },
                _=>notice_icon(&nc,24.0,19.0,a),
            }
            if a.title=="System Volume" {
                self.draw_volume_hud(&nc);
            } else {
                let (title,body)=if a.title=="Keyboard Input Language"{("Input Language",a.body.as_str())}
                    else if a.title=="On Battery Power"{("On Battery",a.body.as_str())}
                    else if a.title=="Network"{a.body.split_once(" · ").unwrap_or(("Network",a.body.as_str()))}
                    else{(a.title.as_str(),a.body.as_str())};
                nc.centered_text(48.0,19.0,&nc.fit(title,11.0,W::Semibold,90.0),11.0,W::Semibold,theme::WHITE,H::Left);
                nc.centered_text(HUD_W-16.0,19.0,&nc.fit(body,10.5,W::Regular,92.0),10.5,W::Regular,if a.title=="Caps Lock"&&a.body=="On"||a.title=="Network"&&a.body.contains("Connected"){theme::GREEN}else{theme::GRAY},H::Right);
            }
        } else {
            let is_water=a.page==2;let is_focus=a.page==1;let is_power=a.title=="Low Battery";
            notice_icon(&nc,22.0,22.0,a);
            let max=if is_water{125.0}else{195.0};
            nc.centered_text(43.0,14.0,&nc.fit(&a.title,11.0,W::Semibold,max),11.0,W::Semibold,theme::WHITE,H::Left);
            nc.centered_text(43.0,30.0,&nc.fit(&a.body,10.5,W::Regular,max),10.5,W::Regular,theme::GRAY,H::Left);
            if is_water {
                if notice_button(&nc,181.0,12.0,62.0,"Later","water-snooze"){self.last_reminder=self.t-self.settings.water_minutes as f64*60.0+600.0;self.notice=None;}
                if notice_button(&nc,250.0,12.0,68.0,"Take a Sip","water-confirm"){self.notice=None;self.drink_water();}
            }else{
                let label=if is_focus{"Start"}else if is_power{"Got It"}else{"View"};
                if notice_button(&nc,257.0,12.0,60.0,label,"notice-action"){
                    self.notice=None;if is_power{return;}if is_focus{self.pomo.running=true;}else if a.page<6{
                        self.show_page(a.page);
                        if a.page==4 {
                            let order=self.todos.display_order(&self.today);
                            if let Some(index)=order.iter().position(|&i|self.todos.items[i].text==a.body && !self.todos.items[i].done_for(&self.today)){
                                self.todo_first=index.min(order.len().saturating_sub(3));
                            }
                        }
                    }else{self.settings_tab=6;self.open_settings();}
                }
            }
        }
    }
    /// v0.5.4：可交互音量条。样式参考 oyenza-studio（MIT）的 HUD 音量条：
    /// 圆角胶囊、填充缓动、拖动时加粗并出现手柄；拖动/滚轮改音量，点喇叭静音。
    pub(crate) fn draw_volume_hud(&mut self, nc: &Card) {
        let (v, muted) = self.volume_ui.unwrap_or((0, false));
        let v = v.min(100);
        // 喇叭：点击静音 / 恢复
        let icon = nc.hit(12.0, 9.0, 28.0, 22.0, "volume-mute");
        if icon.clicked() && nc.live {
            self.volume_ui = Some((v, !muted));
            self.volume_quiet_until = self.t + 0.9;
            self.pending.push(Cmd::ToggleMute);
        }
        // 滑条：拖动 / 点击定位；滚轮微调
        let track = nc.hit_drag(HUD_TRACK_X, 8.0, HUD_TRACK_W, 24.0, "volume-track");
        let mut want: Option<u32> = None;
        if track.dragged() || track.clicked() {
            if let Some(p) = track.interact_pointer_pos() {
                let x = nc.to_local(p).x;
                want = Some((((x - HUD_TRACK_X) / HUD_TRACK_W) * 100.0).clamp(0.0, 100.0).round() as u32);
            }
        } else if nc.live && track.hovered() {
            let dy = nc.ui.ctx().input(|i| i.raw_scroll_delta.y);
            if dy.abs() > 0.5 {
                want = Some(((v as f32 + (dy / 120.0) * 4.0).clamp(0.0, 100.0)).round() as u32);
            }
        }
        if let Some(nv) = want {
            if self.volume_ui.map(|(o, _)| o) != Some(nv) || muted {
                self.volume_ui = Some((nv, false));
                self.volume_quiet_until = self.t + 0.9;
                self.pending.push(Cmd::SetVolume(nv));
            }
        }
        let (v, muted) = self.volume_ui.unwrap_or((v, muted));
        let pressed = track.dragged() || track.is_pointer_button_down_on() || icon.is_pointer_button_down_on();
        let target = if muted { 0.0 } else { v as f32 / 100.0 };
        self.volume_a = crate::anim::approach(self.volume_a, target, if pressed { 34.0 } else { 13.0 }, self.dt);
        let h = if pressed { 7.0 } else { 5.0 };
        let y = 19.0 - h / 2.0;
        nc.fill(HUD_TRACK_X, y, HUD_TRACK_W, h, h / 2.0, Color32::from_white_alpha(if pressed { 46 } else { 34 }));
        if self.volume_a > 0.0 {
            let w = (HUD_TRACK_W * self.volume_a).max(h);
            let col = if muted { Color32::from_gray(150) } else { theme::WHITE };
            nc.fill(HUD_TRACK_X, y, w, h, h / 2.0, col);
            nc.fill(HUD_TRACK_X, y + 0.8, (w - 1.2).max(0.0), h * 0.4, h * 0.2, Color32::from_white_alpha(64));
            if pressed {
                nc.circle(HUD_TRACK_X + w - h / 2.0, y + h / 2.0, h * 0.95, col);
            }
        }
        let label = if muted { "Muted".to_string() } else { format!("{v}%") };
        nc.centered_text(HUD_W - 26.0, 19.0, &label, 10.5, if pressed { W::Semibold } else { W::Regular }, if muted { theme::GRAY } else { theme::WHITE }, H::Center);
    }

    pub(crate) fn drink_water(&mut self) {        let hour = self.current_hour;
        let amount = self.settings.serving_ml;
        let was = self.water.percent();
        let before = self.water.drunk_ml;
        self.water.drink(amount, hour);
        let amount = self.water.drunk_ml.saturating_sub(before);
        self.last_drink = Some((amount, hour));
        self.last_reminder = self.t;
        self.pet.react(PetState::Satisfied);
        self.say(format!("Logged {amount} ml"));
        if was < 100.0 && self.water.percent() >= 100.0 {
            self.alert("Daily water goal reached", "Nice work, keep it up", 2, true);
        }
    }


}

/// 将本地提醒时间规范化，拒绝不存在的日期与超出范围的时分。
pub(crate) fn parse_due(value: &str, today: &str) -> Option<String> {
    let value = value.trim();
    let full = if value.len() == 5 {
        format!("{today} {value}")
    } else {
        value.to_string()
    };
    if !full.is_ascii() || full.len() != 16 {
        return None;
    }
    let b = full.as_bytes();
    if b[4] != b'-' || b[7] != b'-' || b[10] != b' ' || b[13] != b':' {
        return None;
    }
    let year = full[0..4].parse::<u32>().ok()?;
    let month = full[5..7].parse::<usize>().ok()?;
    let day = full[8..10].parse::<u32>().ok()?;
    let hour = full[11..13].parse::<u32>().ok()?;
    let minute = full[14..16].parse::<u32>().ok()?;
    if !(2000..=9999).contains(&year) || !(1..=12).contains(&month) || hour > 23 || minute > 59 {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > days[month - 1] {
        return None;
    }
    Some(full)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn island_size_drag_commits_only_on_release(){
        let ctx=egui::Context::default();let mut value=100;
        let pos=egui::pos2(30.0,30.0);let end=egui::pos2(110.0,30.0);
        let button=|pos,pressed|egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
        let frames=[vec![],vec![egui::Event::PointerMoved(pos)],vec![button(pos,true)],vec![egui::Event::PointerMoved(end)],vec![button(end,false)]];
        for (i,events) in frames.into_iter().enumerate(){
            let _=ctx.run(egui::RawInput{events,screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(320.0,80.0))),..Default::default()},|ctx|{
                egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(20)).show(ctx,|ui|{ui.spacing_mut().slider_width=180.0;island_size_slider(ui,&mut value);});
            });
            if i<4{assert_eq!(value,100);}else{assert_ne!(value,100);}
        }
    }
    #[test]
    fn number_stepper_clicks_follow_steps_and_limits() {
        for (initial,step,x,expected) in [(5u32,1,127.0,6),(5,1,13.0,4),(0,1,13.0,0),(10,1,127.0,10),(9,3,127.0,10),(1,3,13.0,0)] {
            let ctx=egui::Context::default();
            let mut value=initial;
            let pos=egui::pos2(x+20.0,34.0);
            let events=[vec![],vec![egui::Event::PointerMoved(pos)],vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}],vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers:Default::default()}]];
            for events in events {
                let raw=egui::RawInput{screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(200.0,80.0))),events,..Default::default()};
                let _=ctx.run(raw,|ctx|{
                    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(20)).show(ctx,|ui|{
                        ui.horizontal(|ui|{number_stepper(ui,&mut value,0,10,"",step,76.0);});
                    });
                });
            }
            assert_eq!(value,expected,"初值 {initial}，步长 {step}，点击位置 {x}");
        }
    }

    #[test]
    fn number_stepper_fits_with_settings_button_padding() {
        let ctx=egui::Context::default();
        let raw=egui::RawInput{screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(340.0,240.0))),..Default::default()};
        let _=ctx.run(raw,|ctx|{
            egui::CentralPanel::default().frame(egui::Frame::new()).show(ctx,|ui|{
                ui.spacing_mut().button_padding=egui::vec2(10.0,6.0);
                for mut value in [0u32,25,180] {
                    let row=ui.horizontal(|ui|{number_stepper(ui,&mut value,0,180," 分钟",1,76.0);});
                    assert!(row.response.rect.width()<=140.1,"数值控件超出预留宽度：{}",row.response.rect.width());
                }
                for mut value in [25u32,5,15] {setting_number(ui,"计时时长",&mut value,1,180," 分钟",1.0);}
                assert!(ui.min_rect().right()<=340.1,"连续数值行不应撑宽卡片");
            });
        });
    }

    #[test]
    fn settings_groups_keep_every_page_reachable() {
        let groups = ["常规", "音乐与歌词", "专注与饮水", "专注与饮水", "待办与置物", "待办与置物", "通知、邮箱与日程", "通知、邮箱与日程", "通知、邮箱与日程", "关于", "页面管理", "AI 额度"];
        for (tab, expected) in groups.iter().enumerate() {
            let matches: Vec<_> = SETTINGS_GROUPS.iter().filter(|(_,start,end)| (*start..=*end).contains(&tab)).collect();
            assert_eq!(matches.len(), 1, "页面 {tab} 必须且只能属于一个分组");
            assert_eq!(matches[0].0, *expected);
        }
    }

    #[test]
    fn bluetooth_notice_uses_real_connection_and_battery_data(){
        assert_eq!(bluetooth_notice_content("OpenMove by Shokz · 电量 80%"),("OpenMove by Shokz","已连接",Some(80)));
        assert_eq!(bluetooth_notice_content("耳机"),("耳机","已连接",None));
        assert_eq!(bluetooth_notice_content("已断开连接"),("蓝牙设备","已断开",None));
        assert_eq!(bluetooth_notice_content("青龙键盘 · 已断开"),("青龙键盘","已断开",None));
        assert_eq!(bluetooth_notice_content("耳机 · 电量 101%"),("耳机 · 电量 101%","已连接",None));
    }

    #[test]
    fn device_icons_render_as_visible_svg() {
        let icons: &[&[u8]] = &[
            include_bytes!("../assets/device-icons/headphones-round-bold.svg"),
            include_bytes!("../assets/device-icons/speaker-bold.svg"),
            include_bytes!("../assets/device-icons/keyboard-bold.svg"),
            include_bytes!("../assets/device-icons/mouse-bold.svg"),
            include_bytes!("../assets/device-icons/gamepad-bold.svg"),
            include_bytes!("../assets/device-icons/smartphone-bold.svg"),
            include_bytes!("../assets/device-icons/bluetooth-linear.svg"),
        ];
        for bytes in icons {
            let image=egui_extras::image::load_svg_bytes(bytes).unwrap();
            assert!(image.pixels.iter().any(|p|p.a()>0));
        }
    }

    #[test]
    fn due_time_checks_calendar_and_format() {
        assert_eq!(
            parse_due("09:30", "2026-10-29").as_deref(),
            Some("2026-10-29 09:30")
        );
        assert!(parse_due("2026-02-29 12:00", "").is_none());
        assert!(parse_due("2028-02-29 12:00", "").is_some());
        for value in [
            "24:00",
            "12:60",
            "明天中午",
            "2026-13-01 10:00",
            "2026-00-01 10:00",
        ] {
            assert!(parse_due(value, "2026-10-29").is_none());
        }
    }
}

pub(crate) struct TaskDraft {index:usize,original:String,text:String,due:String,error:String,focused:bool,reminder_only:bool,pub(crate) repeat:u8,pub(crate) picker:crate::task_picker::ReminderPicker}
impl App {
    pub(crate) fn open_task_dialog(&mut self,index:usize){
        self.todo_choosing=false;
        if let Some(task)=self.todos.items.get(index){
            let draft=TaskDraft{index,original:task.text.clone(),text:task.text.clone(),due:task.due.clone().unwrap_or_default(),error:String::new(),focused:false,reminder_only:false,repeat:task.repeat,picker:crate::task_picker::ReminderPicker::new(task.due.as_deref())};
            self.show_page(4);self.task_dialog=Some(draft);
        }
    }
    pub(crate) fn open_task_reminder(&mut self,index:usize){
        self.open_task_dialog(index);
        if let Some(d)=self.task_dialog.as_mut(){d.reminder_only=true;d.picker.show();}
    }
    pub(crate) fn draw_task_editor(&mut self,c:&Card){
        if self.task_dialog.as_ref().is_some_and(|draft|draft.picker.open){self.draw_task_reminder(c);return;}
        let reminder_only=self.task_dialog.as_ref().is_some_and(|d|d.reminder_only);
        c.text(15.0,25.0,if reminder_only{"Set Reminder"}else{"Edit To-do"},size::TITLE,W::Bold,theme::WHITE,H::Left);
        c.text(43.0,48.0,if reminder_only{"To-do"}else{"Content"},10.5,W::Regular,theme::GRAY,H::Left);
        c.fill(91.0,33.0,275.0,20.0,6.0,theme::CARD);
        if let Some(d)=self.task_dialog.as_ref().filter(|d|d.reminder_only){
            c.text(97.0,47.0,&c.fit(&d.text,11.5,W::Regular,260.0),11.5,W::Regular,theme::WHITE,H::Left);
        }
        c.text(43.0,72.0,"Reminder",10.5,W::Regular,theme::GRAY,H::Left);
        let time=self.task_dialog.as_ref().map(|d|if d.due.is_empty(){"Click to pick a time".into()}
            else if d.repeat!=0{format!("{} {}",crate::model::repeat_label(d.repeat),d.due.get(11..16).unwrap_or(&d.due))}else{d.due.clone()}).unwrap_or_default();
        if setting_button(c,91.0,58.0,275.0,&time,false,"task-select-reminder"){if let Some(d)=self.task_dialog.as_mut(){d.picker.show();}}
        // 每周重复：七个星期按钮，全不选即一次性待办。
        c.text(43.0,95.0,"Repeat",10.5,W::Regular,theme::GRAY,H::Left);
        self.task_repeat_chips(c,91.0,81.0,25.0,19.0,28.0);
        let repeat=self.task_dialog.as_ref().map(|d|d.repeat).unwrap_or(0);
        let quick=if repeat==crate::model::REPEAT_ALL{"No Repeat"}else{"Daily"};
        if setting_button(c,289.0,81.0,77.0,quick,false,"task-repeat-all"){
            if let Some(d)=self.task_dialog.as_mut(){d.repeat=if d.repeat==crate::model::REPEAT_ALL{0}else{crate::model::REPEAT_ALL};}
        }
        if let Some(d)=&self.task_dialog {
            let summary=if d.repeat!=0{crate::model::repeat_label(d.repeat)}else{"Once".into()};
            let hint=if d.error.is_empty(){summary.as_str()}else{&d.error};
            c.text(43.0,117.0,&c.fit(hint,10.5,W::Regular,if reminder_only{140.0}else{154.0}),10.5,if d.error.is_empty(){W::Regular}else{W::Semibold},if d.error.is_empty(){theme::GRAY}else{theme::RED},H::Left);
        }
        if reminder_only {
            let has_reminder=self.task_dialog.as_ref().and_then(|d|self.todos.items.get(d.index)).is_some_and(|task|task.due.is_some());
            let skip=if has_reminder{"Remove Reminder"}else if repeat!=0{"Repeat Only"}else{"No Reminder"};
            if setting_button(c,188.0,104.0,88.0,skip,false,"task-reminder-skip"){
                if let Some(d)=self.task_dialog.as_mut(){d.due.clear();}
                self.save_task_editor();
            }
        }else if setting_button(c,202.0,104.0,74.0,"Cancel",false,"task-cancel"){self.task_dialog=None;}
        if setting_button(c,286.0,104.0,80.0,if reminder_only{"Pick Time"}else{"Save"},true,"task-save"){
            if reminder_only{if let Some(d)=self.task_dialog.as_mut(){d.picker.show();}}else{self.save_task_editor();}
        }
    }
    pub(crate) fn task_editor_inputs(&mut self,ui:&mut egui::Ui,origin:egui::Pos2,cmds:&mut Vec<Cmd>){
        let Some(d)=self.task_dialog.as_mut() else{return};
        if d.reminder_only || d.picker.open{return;}
        let name=ui.put(Rect::from_min_size(origin+egui::vec2(97.0,35.0),egui::vec2(260.0,17.0)),egui::TextEdit::singleline(&mut d.text).vertical_align(egui::Align::Center).id(egui::Id::new("task-name-edit")).frame(false).font(theme::font(11.5,W::Regular)));
        let first_focus=!d.focused;
        if first_focus{name.request_focus();d.focused=true;cmds.push(Cmd::Focus);}
        // 新建内容与下一步共用这一帧，避免同一次回车又提交了空提醒。
        if !first_focus && ui.input(|i|i.key_pressed(egui::Key::Enter)){self.save_task_editor();}
    }
    /// 七个星期按钮（一…日），点一下切换；全不选即一次性待办。
    fn task_repeat_chips(&mut self,c:&Card,x:f32,y:f32,w:f32,h:f32,step:f32){
        let repeat=self.task_dialog.as_ref().map(|d|d.repeat).unwrap_or(0);
        for (day,name) in crate::model::WEEKDAY_NAMES.iter().enumerate() {
            let on=repeat&(1<<day)!=0;
            let cx=x+day as f32*step;
            let (r,b)=crate::paint::capsule(c,cx,y,w,h,&format!("task-repeat-{day}"),if on{Color32::from_rgb(39,94,150)}else{theme::BTN},theme::BTN_HOVER);
            b.centered_text(cx+w/2.0,y+h/2.0,name,if h<18.0{9.5}else{10.5},W::Semibold,if on{theme::WHITE}else{theme::GRAY},H::Center);
            if r.on_hover_text(format!("Every {name}")).clicked(){
                if let Some(d)=self.task_dialog.as_mut(){d.repeat^=1<<day;d.error.clear();}
            }
        }
    }
    fn draw_task_reminder(&mut self,c:&Card){
        use crate::task_picker::PickerAction;
        let Some(d)=self.task_dialog.as_mut().filter(|d|d.picker.open) else{return};
        let has_reminder=self.todos.items.get(d.index).is_some_and(|task|task.due.is_some());
        c.text(15.0,25.0,"To-do",size::TITLE,W::Bold,theme::WHITE,H::Left);
        c.text(44.0,25.0,"List",size::TITLE,W::Bold,theme::GREEN,H::Left);
        let (done,total)=self.todos.progress(&self.today);
        c.text(42.0,61.0,&format!("{done}/{total}"),29.0,W::Bold,theme::WHITE,H::Left);
        // 左侧：每周重复。选了星期几后，提醒只按时间、在这些日子到点提醒。
        d.picker.repeat_note=Some(if d.repeat!=0{format!("{} · Remind on time",crate::model::repeat_label(d.repeat))}else{String::new()});
        let action=d.picker.draw(c,&d.text,has_reminder);
        let focus=std::mem::take(&mut d.picker.focus);
        self.task_repeat_chips(c,41.0,84.0,17.0,16.0,19.5);
        let Some(d)=self.task_dialog.as_mut() else{return};
        if focus{self.pending.push(Cmd::Focus);}
        match action{
            Some(PickerAction::Save)=>{d.due=d.picker.due();d.picker.open=false;if d.reminder_only{self.save_task_editor();}},
            Some(PickerAction::Skip)=>{d.due.clear();d.picker.open=false;if d.reminder_only{self.save_task_editor();}},
            Some(PickerAction::Cancel)=>{d.picker.open=false;if d.reminder_only{self.task_dialog=None;}},
            None=>{},
        }
    }
    fn save_task_editor(&mut self){
        let Some(mut d)=self.task_dialog.take() else{return};
        let due=if d.due.trim().is_empty(){Some(None)}else{parse_due(&d.due,&self.today).map(Some)};
        if d.text.trim().is_empty(){d.error="Please enter the task.".into();}
        else if due.is_none(){d.error="Time must be 18:30 or a full date and time.".into();}
        else if self.todos.items.get(d.index).is_none_or(|t|t.text!=d.original){d.error="Task has changed. Cancel and reopen it.".into();}
        else{
            let item=&mut self.todos.items[d.index];
            if item.repeat!=d.repeat{item.done_on=None;}
            item.text=d.text.trim().into();item.due=due.unwrap();item.reminded=false;item.reminded_on=None;item.repeat=d.repeat;
            self.todos.save();self.say("To-do saved");return;
        }
        self.task_dialog=Some(d);
    }
}

fn notice_button(c:&Card,x:f32,y:f32,w:f32,text:&str,id:&str)->bool {
    let (r,b)=crate::paint::capsule(c,x,y,w,20.0,id,Color32::from_gray(35),theme::BTN_HOVER);
    b.centered_text(x+w/2.0,y+10.0,text,10.5,W::Semibold,theme::WHITE,H::Center);
    r.clicked()
}
fn setting_button(c:&Card,x:f32,y:f32,w:f32,text:&str,on:bool,id:&str)->bool {
    let (r,b)=crate::paint::capsule(c,x,y,w,19.0,id,if on{Color32::from_rgb(39,94,150)}else{theme::BTN},theme::BTN_HOVER);
    b.text(x+w/2.0,y+13.3,text,10.5,W::Semibold,theme::WHITE,H::Center);
    r.clicked()
}
impl App {
    pub(crate) fn page_settings(&mut self,c:&Card) {
        let help=match self.settings_category{0=>"Appearance & System",1=>"Adjust water amount and reminder times",2=>"Click the minutes to type, or use +/- to fine-tune",_=>"Notifications"};
        c.text(44.0,44.0,help,10.5,W::Regular,theme::GRAY,H::Left);
        let before=format!("{:?}",self.settings);
        match self.settings_category {
            0=>{
                let choices=[("Mascot",self.settings.pet),("Minimal Notch",self.settings.minimal),("System HUD",self.settings.system_hud),("Hide in Fullscreen",self.settings.hide_fullscreen)];
                for (i,(name,on)) in choices.iter().enumerate(){let x=42.0+(i%2) as f32*169.0;let y=57.0+(i/2) as f32*27.0;let text=format!("{}  {}",name,if *on{"On"}else{"Off"});if setting_button(c,x,y,158.0,&text,*on,&format!("general-{i}")){match i{0=>self.set_pet(!self.settings.pet),1=>self.set_minimal(!self.settings.minimal),2=>self.settings.system_hud=!self.settings.system_hud,_=>self.settings.hide_fullscreen=!self.settings.hide_fullscreen}}}
                c.text(42.0,116.0,"Daozai stays visible when collapsed; minimal mode keeps a thin line",10.5,W::Regular,theme::GRAY,H::Left);
            }
            1=>{
                let rows=[("Per Drink",format!("{} ml",self.settings.serving_ml)),("Interval",format!("{} min",self.settings.water_minutes)),("Goal",format!("{:.1} L",self.water.goal_ml as f32/1000.0))];
                for (i,(label,value)) in rows.iter().enumerate(){let y=53.0+i as f32*23.0;c.text(44.0,y+13.0,label,10.5,W::Regular,theme::GRAY,H::Left);c.text(148.0,y+13.0,value,10.5,W::Semibold,theme::WHITE,H::Center);
                    if setting_button(c,90.0,y,25.0,"−",false,&format!("water-minus-{i}")){match i{0=>self.settings.serving_ml=self.settings.serving_ml.saturating_sub(50).max(50),1=>self.settings.water_minutes=self.settings.water_minutes.saturating_sub(5).max(5),_=>{self.water.goal_ml=self.water.goal_ml.saturating_sub(250).max(500);self.water.save();}}}
                    if setting_button(c,180.0,y,25.0,"+",false,&format!("water-plus-{i}")){match i{0=>self.settings.serving_ml=(self.settings.serving_ml+50).min(1000),1=>self.settings.water_minutes=(self.settings.water_minutes+5).min(180),_=>{self.water.goal_ml=(self.water.goal_ml+250).min(6000);self.water.save();}}}}
                c.text(223.0,66.0,"Reminder Hours",10.5,W::Semibold,theme::WHITE,H::Left);
                for i in 0..2 {let y=73.0+i as f32*23.0;let hour=if i==0{self.settings.water_start}else{self.settings.water_end};c.text(250.0,y+13.0,&format!("{:02}:00",hour),10.5,W::Bold,theme::WHITE,H::Center);
                    if setting_button(c,287.0,y,25.0,"−",false,&format!("hour-minus-{i}")){if i==0{self.settings.water_start=(hour+23)%24}else{self.settings.water_end=(hour+23)%24}}
                    if setting_button(c,318.0,y,25.0,"+",false,&format!("hour-plus-{i}")){if i==0{self.settings.water_start=(hour+1)%24}else{self.settings.water_end=(hour+1)%24}}}
            }
            2=>{
                self.focus_editing=false;
                for (i,name) in ["Focus","Break"].iter().enumerate(){
                    let y=53.0+i as f32*23.0;
                    c.text(48.0,y+13.0,name,10.5,W::Regular,theme::GRAY,H::Left);
                    let value=if i==0{&mut self.settings.focus_minutes}else{&mut self.settings.rest_minutes};
                    let max=if i==0{crate::model::FOCUS_MAX_MINUTES}else{60};let rect=c.rect(168.0,y,76.0,19.0);
                    let mut ui=egui::Ui::new(c.ui.ctx().clone(),egui::Id::new(("focus-duration",i)),egui::UiBuilder::new().layer_id(c.ui.layer_id()).max_rect(rect).style(c.ui.style().clone()));
                    ui.set_clip_rect(c.painter.clip_rect());ui.set_opacity(c.painter.opacity());if !c.live{ui.disable();}
                    ui.spacing_mut().interact_size=rect.size();ui.spacing_mut().button_padding=egui::vec2(2.0,1.0);
                    ui.style_mut().drag_value_text_style=egui::TextStyle::Body;
                    ui.style_mut().text_styles.insert(egui::TextStyle::Body,c.font(11.0,W::Bold));
                    ui.style_mut().explanation_tooltips=false;
                    ui.visuals_mut().widgets.inactive.weak_bg_fill=Color32::TRANSPARENT;
                    let drag=egui::DragValue::new(value).range(1..=max).update_while_editing(false);
                    let drag=if i==0{hours_drag(drag)}else{drag.suffix(" min")};
                    let field=ui.put(rect,drag).on_hover_text(if i==0{"Click to type: 90, 1h30 or 2.5h"}else{"Click to enter minutes"});
                    if field.gained_focus(){self.pending.push(Cmd::Focus);}
                    self.focus_editing|=field.has_focus();
                    let step=if i==0&&*value>=60{15}else if i==0{5}else{1};
                    if setting_button(c,131.0,y,27.0,"−",false,&format!("focus-minus-{i}")){*value=value.saturating_sub(step).max(1);}
                    if setting_button(c,255.0,y,27.0,"+",false,&format!("focus-plus-{i}")){*value=(*value+step).min(max);}
                }
                c.text(315.0,93.0,"Applies next time",10.5,W::Regular,theme::GRAY,H::Center);
            }
            _=>{
                if setting_button(c,43.0,56.0,152.0,if self.settings.notifications{"Alerts · On"}else{"Alerts · Off"},self.settings.notifications,"set-notice"){self.settings.notifications=!self.settings.notifications;}
                if setting_button(c,212.0,56.0,153.0,if self.settings.sound{"Sound · On"}else{"Sound · Off"},self.settings.sound,"set-sound"){self.settings.sound=!self.settings.sound;}
                if setting_button(c,43.0,84.0,152.0,if self.settings.read_notifications{"Disconnect System Notifications"}else{"Connect System Notifications"},self.settings.read_notifications,"set-listener"){self.pending.push(if self.settings.read_notifications{Cmd::DisconnectNotifications}else{Cmd::RequestNotifications});}
                let status=c.fit(&self.listener_status,10.5,W::Regular,170.0);c.text(210.0,97.0,&status,10.5,W::Regular,theme::GRAY,H::Left);
            }
        }
        if before!=format!("{:?}",self.settings){self.settings.save();self.last_reminder=self.t;}
    }
}

fn switch(ui:&mut egui::Ui,value:&mut bool)->bool {
    let (rect,response)=ui.allocate_exact_size(egui::vec2(34.0,20.0),egui::Sense::click());
    if response.clicked(){*value=!*value;}
    let fill=if *value{Color32::from_rgb(48,184,92)}else{settings_color(&ui.ctx().clone(),Color32::from_rgb(82,82,87),Color32::from_rgb(203,207,214))};
    ui.painter().rect_filled(rect,10.0,fill);
    let x=if *value{rect.right()-10.0}else{rect.left()+10.0};
    ui.painter().circle_filled(egui::pos2(x,rect.center().y+0.5),7.5,Color32::from_black_alpha(35));
    ui.painter().circle_filled(egui::pos2(x,rect.center().y),7.0,Color32::WHITE);
    response.clicked()
}
/// 设置页版式：标签列宽、开关尺寸、控件与卡片右边缘之间的固定留白。
pub(crate) const SETTINGS_LABEL_W: f32 = 80.0;
const SETTINGS_SWITCH_W: f32 = 34.0;
const SETTINGS_ROW_GAP: f32 = 16.0;

/// 开关行：左侧标题 + 说明（宽度固定、自动换行），右侧开关，长说明也不会把开关挤出卡片。
pub(crate) fn setting_row(ui:&mut egui::Ui,title:&str,description:&str,value:&mut bool){
    let ctx=&ui.ctx().clone();
    ui.add_space(2.0);
    ui.horizontal(|ui|{
        let text_w=(ui.available_width()-SETTINGS_SWITCH_W-SETTINGS_ROW_GAP).max(120.0);
        ui.vertical(|ui|{
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y=2.0;
            ui.add(egui::Label::new(egui::RichText::new(title).size(13.0).color(settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41)))).wrap());
            if !description.is_empty(){ui.add(egui::Label::new(egui::RichText::new(description).size(11.5).color(settings_color(ctx,Color32::from_gray(175),Color32::from_gray(90)))).wrap());}
        });
        // add_space 之后还会再插入一次 item_spacing.x，这里一并扣掉，开关才不会每行往外挤一点。
        ui.add_space((ui.available_width()-SETTINGS_SWITCH_W-ui.spacing().item_spacing.x).max(0.0));
        switch(ui,value);
    });
    ui.add_space(2.0);
}

/// 设置页统一的分段选择器（灰色底框 + 选中块）。返回值为是否发生了切换。
pub(crate) fn settings_segmented<T:PartialEq+Copy>(ui:&mut egui::Ui,value:&mut T,options:&[(T,&str)],item_w:f32)->bool{
    let ctx=&ui.ctx().clone();
    let mut changed=false;
    egui::Frame::new().fill(settings_color(ctx,Color32::from_rgb(28,28,30),Color32::from_rgb(230,233,238)))
        .corner_radius(8.0).inner_margin(3).show(ui,|ui|{
            ui.horizontal(|ui|{
                ui.spacing_mut().item_spacing.x=2.0;
                for (option,label) in options {
                    let selected=*value==*option;
                    let text=egui::RichText::new(*label).size(12.0).color(if selected{settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41))}else{settings_color(ctx,Color32::from_gray(200),Color32::from_gray(70))});
                    let button=egui::Button::new(text)
                        .fill(if selected{settings_color(ctx,Color32::from_rgb(72,72,76),Color32::WHITE)}else{Color32::TRANSPARENT})
                        .stroke(egui::Stroke::NONE).corner_radius(6.0);
                    let width=item_w.max(ctx.fonts(|f|f.layout_no_wrap(label.to_string(),egui::FontId::proportional(12.0),Color32::WHITE).size().x)+20.0);
                    if ui.add_sized([width,26.0],button).clicked() && !selected {*value=*option;changed=true;}
                }
            });
        });
    changed
}

/// 选项行：标签列之后紧跟控件，同一卡片里所有控件左缘对齐成一条竖线。
pub(crate) fn settings_control_row_right(ui:&mut egui::Ui,title:&str,controls:impl FnOnce(&mut egui::Ui))->egui::Response{
    settings_control_row(ui,title,SETTINGS_LABEL_W,|ui|ui.horizontal(|ui|controls(ui)).response)
}

/// 设置项下方的说明文字。
fn island_size_slider(ui:&mut egui::Ui,value:&mut u32)->egui::Response {
    let id=ui.make_persistent_id("island-size-draft");
    let mut draft=ui.data(|d|d.get_temp::<u32>(id)).unwrap_or(*value);
    let response=ui.add(egui::Slider::new(&mut draft,crate::model::ISLAND_SIZE_MIN..=crate::model::ISLAND_SIZE_MAX)
        .step_by(crate::model::ISLAND_SIZE_STEP as f64).suffix("%"));
    if ui.input(|i|i.pointer.primary_down()) {
        if response.changed(){ui.data_mut(|d|d.insert_temp(id,draft));}
    }else{
        *value=crate::model::island_size_value(draft);
        ui.data_mut(|d|d.remove::<u32>(id));
    }
    response
}

pub(crate) fn settings_hint(ui:&mut egui::Ui,text:&str){
    let ctx=&ui.ctx().clone();
    ui.add(egui::Label::new(egui::RichText::new(text).size(11.5).color(settings_color(ctx,Color32::from_gray(160),Color32::from_gray(105)))).wrap());
}
pub(crate) fn settings_card(ui:&mut egui::Ui,add:impl FnOnce(&mut egui::Ui)) {
    let ctx=&ui.ctx().clone();
    egui::Frame::new().fill(settings_color(ctx,Color32::from_rgb(35,35,38),Color32::from_rgb(255,255,255)))
        .corner_radius(10.0).inner_margin(egui::Margin::symmetric(16,12))
        .show(ui,|ui|{ui.set_min_width(ui.available_width());ui.spacing_mut().item_spacing.y=8.0;add(ui);});
}
pub(crate) fn settings_label(ui:&mut egui::Ui,text:&str){
    let ctx=&ui.ctx().clone();
    ui.add_space(9.0);
    ui.label(egui::RichText::new(text).size(11.5).color(settings_color(ctx,Color32::from_gray(185),Color32::from_gray(70))));
    ui.add_space(2.0);
}
fn support_author(ui:&mut egui::Ui){
    let id=egui::Id::new("support-author-open");
    let open=ui.ctx().data(|data|data.get_temp::<bool>(id).unwrap_or(false));
    if !open {release_support_codes(ui.ctx());}
    let dark=ui.visuals().dark_mode;
    let accent=if dark{Color32::from_rgb(182,163,245)}else{Color32::from_rgb(105,77,163)};
    ui.scope(|ui|{
    ui.set_min_height(225.0);
    if !open {
    egui::Frame::new().fill(if dark{Color32::from_rgb(42,37,53)}else{Color32::from_rgb(239,233,250)})
        .corner_radius(10.0).show(ui,|ui|{
            ui.set_min_width(ui.available_width());
            ui.vertical_centered(|ui|{
                ui.add(egui::Image::new(egui::include_image!("../assets/support/daozai-coffee.svg")).fit_to_exact_size(egui::vec2(211.0,110.0)));
            });
        });
    ui.add_space(9.0);
    ui.label(egui::RichText::new("Maintained by Xiao Tang · Daozai keeps you company").size(10.5).color(accent));
    ui.label(egui::RichText::new("Buy Xiao Tang a Coffee").size(19.0).strong());
    ui.label("Daozai is here for your everyday life.\nYour support helps Xiao Tang keep making it better.");
    } else {
        let ctx=ui.ctx().clone();
        // 只解码一次并裁出二维码区域，按实际显示像素缩小：原图解码后两张合计约 17MB，缩小后不到 1MB。
        let pixels=((180.0*ctx.pixels_per_point()).ceil() as u32).clamp(180,600);
        ui.columns(2,|columns|{
            for (i,(name,bytes,crop,color)) in [
                ("WeChat Pay",include_bytes!("../assets/support/support-wechat-original.jpg").as_slice(),[330.0,370.0,600.0],Color32::from_rgb(5,160,80)),
                ("Alipay",include_bytes!("../assets/support/support-alipay-original.jpg").as_slice(),[200.0,660.0,860.0],Color32::from_rgb(22,120,255)),
            ].into_iter().enumerate(){
                let texture_id=egui::Id::new(("support-author-qr",i));
                let texture=ctx.data_mut(|data|data.get_temp::<egui::TextureHandle>(texture_id)).filter(|texture|texture.size()==[pixels as usize;2]).unwrap_or_else(||{
                    let image=support_code_image(bytes,crop,pixels);
                    let texture=ctx.load_texture(name,image,egui::TextureOptions::LINEAR);
                    ctx.data_mut(|data|data.insert_temp(texture_id,texture.clone()));texture
                });
                let uv=egui::Rect::from_min_max(egui::pos2(0.0,0.0),egui::pos2(1.0,1.0));
                columns[i].vertical_centered(|ui|{
                    ui.label(egui::RichText::new(name).color(color).strong());
                    let side=ui.available_width().min(180.0);
                    ui.add(egui::Image::new((texture.id(),egui::vec2(side,side))).uv(uv));
                    ui.label(egui::RichText::new(if i==0{"Scan with WeChat"}else{"Scan with Alipay"}).size(11.0));
                });
            }
        });
    }
    });
    let button=egui::Button::new(egui::RichText::new(if open{"Back"}else{"Buy a Coffee · Support the Author"}).color(Color32::from_rgb(33,27,51)))
        .fill(Color32::from_rgb(182,163,245)).corner_radius(8.0);
    if ui.add_sized([ui.available_width(),32.0],button).clicked(){
        ui.ctx().data_mut(|data|data.insert_temp(id,!open));
    }
    ui.add_space(7.0);
    ui.label(egui::RichText::new("Entirely optional · Thanks for every bit of support · No features are locked").size(11.0).color(ui.visuals().weak_text_color()));
}
pub(crate) fn settings_control_row(ui:&mut egui::Ui,title:&str,label_width:f32,controls:impl FnOnce(&mut egui::Ui)->egui::Response)->egui::Response{
    ui.horizontal(|ui|{
        // 同一张卡片里的控件左缘对齐：标签列至少 SETTINGS_LABEL_W。
        let (label_rect,_)=ui.allocate_exact_size(egui::vec2(label_width.max(SETTINGS_LABEL_W),30.0),egui::Sense::hover());
        let content=controls(ui);
        ui.painter().text(egui::pos2(label_rect.left(),content.rect.center().y),egui::Align2::LEFT_CENTER,title,egui::TextStyle::Body.resolve(ui.style()),ui.visuals().text_color());
        content
    }).inner
}
fn setting_number<N:egui::emath::Numeric>(ui:&mut egui::Ui,title:&str,value:&mut N,min:N,max:N,suffix:&str,speed:f64)->bool{
    let mut changed=false;
    settings_control_row(ui,title,80.0,|ui|{
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
            ui.allocate_ui_with_layout(egui::vec2(140.0,30.0),egui::Layout::left_to_right(egui::Align::Center),|ui|{
                changed=number_stepper(ui,value,min,max,suffix,N::from_f64(speed),76.0);
            });
        }).response
    });
    changed
}
/// Minutes shown as "45 min", "2h" or "1h 30m".
pub(crate) fn minutes_text(m:u32)->String{
    if m<60{format!("{m} min")}else if m%60==0{format!("{}h",m/60)}else{format!("{}h {:02}m",m/60,m%60)}
}
/// Accepts "90", "90 min", "1:30", "1h30", "1h 30m", "2.5h" and returns minutes.
pub(crate) fn parse_minutes(s:&str)->Option<f64>{
    let s=s.trim().to_lowercase().replace(' ',"");
    let s=s.trim_end_matches("min").trim_end_matches('m');
    if s.is_empty(){return None;}
    if let Some((h,m))=s.split_once(':'){return Some(h.parse::<f64>().ok()?*60.0+if m.is_empty(){0.0}else{m.parse::<f64>().ok()?});}
    if let Some((h,m))=s.split_once('h'){
        let h=if h.is_empty(){0.0}else{h.parse::<f64>().ok()?};
        let m=if m.is_empty(){0.0}else{m.parse::<f64>().ok()?};
        return Some((h*60.0+m).round());
    }
    s.parse::<f64>().ok()
}
pub(crate) fn hours_drag(drag:egui::DragValue<'_>)->egui::DragValue<'_>{
    drag.custom_formatter(|v,_|minutes_text(v.round().max(0.0) as u32)).custom_parser(parse_minutes)
}
fn setting_hours(ui:&mut egui::Ui,title:&str,value:&mut u32,min:u32,max:u32)->bool{
    let mut changed=false;
    settings_control_row(ui,title,80.0,|ui|{
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
            ui.allocate_ui_with_layout(egui::vec2(140.0,30.0),egui::Layout::left_to_right(egui::Align::Center),|ui|{
                changed=number_stepper_ex(ui,value,min,max,"",if *value>=60{15}else{5},76.0,true);
            });
        }).response
    });
    changed
}
fn number_stepper<N:egui::emath::Numeric>(ui:&mut egui::Ui,value:&mut N,min:N,max:N,suffix:&str,step:N,field_width:f32)->bool{
    number_stepper_ex(ui,value,min,max,suffix,step,field_width,false)
}
#[allow(clippy::too_many_arguments)]
fn number_stepper_ex<N:egui::emath::Numeric>(ui:&mut egui::Ui,value:&mut N,min:N,max:N,suffix:&str,step:N,field_width:f32,hours:bool)->bool{
    let ctx=&ui.ctx().clone();
    let mut changed=false;
    ui.scope(|ui|{
        ui.spacing_mut().item_spacing.x=6.0;
        ui.spacing_mut().button_padding=egui::vec2(4.0,2.0);
        let button=|text|egui::Button::new(egui::RichText::new(text).size(16.0).color(settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41))))
            .fill(settings_color(ctx,Color32::from_rgb(48,48,51),Color32::from_rgb(229,232,238))).stroke(egui::Stroke::NONE).corner_radius(13.0);
        let decrease=ui.add_enabled(*value>min,button("−").min_size(egui::vec2(26.0,26.0))).on_hover_text("Decrease");
        if decrease.clicked(){
            *value=N::from_f64((value.to_f64()-step.to_f64()).max(min.to_f64()));changed=true;
        }
        let drag=egui::DragValue::new(value).range(min..=max).speed(step.to_f64()).suffix(suffix).fixed_decimals(if N::INTEGRAL{0}else{2});
        let field=ui.add_sized([field_width,28.0],if hours{hours_drag(drag)}else{drag});
        changed|=field.changed();
        if ui.add_enabled(*value<max,button("+").min_size(egui::vec2(26.0,26.0))).on_hover_text("Increase").clicked(){
            *value=N::from_f64((value.to_f64()+step.to_f64()).min(max.to_f64()));changed=true;
        }
    });
    changed
}
fn setting_choices(ui:&mut egui::Ui,title:&str,value:&mut u32,values:[u32;3],unit:&str){
    let labels=values.map(|v|format!("{v} {unit}"));
    let options:Vec<(u32,&str)>=values.iter().zip(labels.iter()).map(|(v,l)|(*v,l.as_str())).collect();
    settings_control_row_right(ui,title,|ui|{settings_segmented(ui,value,&options,62.0);});
}
const SETTINGS_GROUPS: [(&str,usize,usize);8] = [
    ("General",0,0), ("Pages",10,10), ("Music & Lyrics",1,1), ("Focus & Water",2,3),
    ("To-dos & Shelf",4,5), ("Notifications, Mail & Schedule",6,8), ("AI Quota",11,11), ("About",9,9),
];

impl App {
    pub fn settings_ui(&mut self,ctx:&egui::Context,autostart:bool){
        if !self.settings_open{release_support_codes(ctx);}
        self.updates.poll(ctx);
        self.updates.dialog_ui(ctx);
        if !self.settings_open{return;}
        let id=egui::ViewportId::from_hash_of("daozai-settings");
        let focus_requested=std::mem::take(&mut self.settings_focus_requested);
        ctx.show_viewport_immediate(id,egui::ViewportBuilder::default().with_title("Daozai Settings").with_icon(theme::app_icon()).with_inner_size([690.0,560.0]).with_min_inner_size([600.0,450.0]).with_visible(false).with_active(false).with_window_level(egui::WindowLevel::Normal),|ctx,_|{
            let dark=self.settings.settings_theme.dark(crate::settings_theme::system_dark(ctx));
            let original_style=ctx.style();
            let mut style=(*original_style).clone();
            style.visuals=if dark{egui::Visuals::dark()}else{egui::Visuals::light()};
            ctx.set_style(style);
            ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(if dark{egui::SystemTheme::Dark}else{egui::SystemTheme::Light}));
            if self.settings.settings_theme==crate::settings_theme::SettingsTheme::System {
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
            }
            if focus_requested {
                // Glow 在首帧交换缓冲后执行窗口命令，避免先显示白色空窗口。
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            if ctx.input(|i|i.viewport().close_requested()){self.settings_open=false;}
            let before=format!("{:?}",self.settings);
            egui::SidePanel::left("settings-sidebar").resizable(false).exact_width(180.0).frame(egui::Frame::new().fill(settings_color(ctx,Color32::from_rgb(29,29,32),Color32::from_rgb(237,239,243))).stroke(egui::Stroke::new(1.0,settings_color(ctx,Color32::from_rgb(42,42,45),Color32::from_rgb(216,219,225)))).inner_margin(egui::Margin::symmetric(10,12))).show(ctx,|ui|{
                ui.add_space(5.0);
                ui.label(egui::RichText::new("Settings").size(11.0).color(settings_color(ctx,Color32::from_gray(150),Color32::from_gray(105))));
                ui.add_space(7.0);
                for (name,start,end) in SETTINGS_GROUPS {
                    if start==9 {ui.add_space((ui.available_height()-38.0).max(0.0));}
                    let selected=(start..=end).contains(&self.settings_tab);
                    let (rect,response)=ui.allocate_exact_size(egui::vec2(ui.available_width(),36.0),egui::Sense::click());
                    if response.clicked() && !selected{self.settings_tab=start;}
                    if selected || response.hovered(){
                        ui.painter().rect_filled(rect,7.0,if selected{settings_color(ctx,Color32::from_rgb(43,43,46),Color32::from_rgb(214,221,232))}else{settings_color(ctx,Color32::from_rgb(30,30,33),Color32::from_rgb(225,230,238))});
                    }
                    let icon=match start {
                        0=>egui::include_image!("../assets/settings-icons/system/general.svg"),
                        11=>egui::include_image!("../assets/settings-icons/system/quota.svg"),
                        10=>egui::include_image!("../assets/settings-icons/system/pages.svg"),
                        1=>egui::include_image!("../assets/settings-icons/system/music.svg"),
                        2=>egui::include_image!("../assets/settings-icons/system/focus.svg"),
                        4=>egui::include_image!("../assets/settings-icons/system/tasks.svg"),
                        6=>egui::include_image!("../assets/settings-icons/system/notifications.svg"),
                        _=>egui::include_image!("../assets/settings-icons/system/about.svg"),
                    };
                    let icon_bg=egui::Rect::from_center_size(egui::pos2(rect.left()+21.0,rect.center().y),egui::vec2(25.0,25.0));
                    egui::Image::new(icon).paint_at(ui,icon_bg);
                    ui.painter().text(egui::pos2(rect.left()+40.0,rect.center().y),egui::Align2::LEFT_CENTER,name,egui::FontId::proportional(13.0),if selected{settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41))}else{settings_color(ctx,Color32::from_gray(225),Color32::from_gray(55))});
                    ui.add_space(3.0);
                }
            });
            egui::CentralPanel::default().frame(egui::Frame::new().fill(settings_color(ctx,Color32::from_rgb(25,25,27),Color32::from_rgb(246,247,249))).inner_margin(egui::Margin::symmetric(22,16))).show(ctx,|ui|{
                ui.spacing_mut().item_spacing=egui::vec2(8.0,5.0);
                ui.spacing_mut().button_padding=egui::vec2(10.0,6.0);
                ui.style_mut().text_styles.insert(egui::TextStyle::Small,egui::FontId::proportional(12.0));
                ui.visuals_mut().override_text_color=Some(settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41)));
                ui.visuals_mut().extreme_bg_color=settings_color(ctx,Color32::from_rgb(25,25,28),Color32::from_rgb(244,246,249));
                ui.visuals_mut().widgets.inactive.bg_fill=settings_color(ctx,Color32::from_rgb(50,50,54),Color32::from_rgb(228,231,236));
                ui.visuals_mut().widgets.hovered.bg_fill=settings_color(ctx,Color32::from_rgb(65,65,71),Color32::from_rgb(216,222,231));
                // 统一按钮：高度 28、圆角 7、内边距 12×5，无描边；默认按钮也套用这套样式。
                ui.spacing_mut().button_padding=egui::vec2(12.0,5.0);
                ui.spacing_mut().interact_size.y=28.0;
                {
                    let fill=settings_color(ctx,Color32::from_rgb(50,50,54),Color32::from_rgb(228,231,236));
                    let hover=settings_color(ctx,Color32::from_rgb(65,65,71),Color32::from_rgb(216,222,231));
                    let widgets=&mut ui.visuals_mut().widgets;
                    for (w,f) in [(&mut widgets.inactive,fill),(&mut widgets.hovered,hover),(&mut widgets.active,hover)] {
                        w.corner_radius=egui::CornerRadius::same(7);
                        w.weak_bg_fill=f;
                        w.bg_stroke=egui::Stroke::NONE;
                    }
                }
                ui.visuals_mut().selection.bg_fill=settings_color(ctx,Color32::from_rgb(45,105,165),Color32::from_rgb(209,226,248));
                let titles=["General","Music & Lyrics","Focus","Water","To-do","Shelf","Notifications","Mail","Schedule","About","Pages","AI Quota"];
                ui.label(egui::RichText::new(titles[self.settings_tab.min(11)]).size(18.0).strong().color(settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41))));
                if let Some((_,start,end))=SETTINGS_GROUPS.iter().find(|(_,start,end)|(*start..=*end).contains(&self.settings_tab)) {
                    if start!=end {
                        ui.add_space(7.0);
                        egui::Frame::new().fill(settings_color(ctx,Color32::from_rgb(33,33,36),Color32::from_rgb(232,235,240))).corner_radius(8.0).inner_margin(3).show(ui,|ui|{
                            ui.spacing_mut().item_spacing.x=2.0;
                            ui.horizontal(|ui|{
                                for index in *start..=*end {
                                    let selected=self.settings_tab==index;
                                    let button=egui::Button::new(egui::RichText::new(titles[index]).size(12.0).color(if selected{settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41))}else{settings_color(ctx,Color32::from_gray(210),Color32::from_gray(55))}))
                                        .fill(if selected{settings_color(ctx,Color32::from_rgb(70,70,74),Color32::from_rgb(205,215,229))}else{Color32::TRANSPARENT})
                                        .stroke(egui::Stroke::NONE).corner_radius(6.0);
                                    if ui.add_sized([68.0,26.0],button).clicked(){self.settings_tab=index;}
                                }
                            });
                        });
                    }
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical().id_salt(("settings-scroll",self.settings_tab)).auto_shrink([false,false]).show(ui,|ui|{
                    // 右侧给滚动条留出空隙，卡片和开关不会被它压住。
                    ui.set_width((ui.available_width()-12.0).max(240.0));
                    match self.settings_tab {
                    11=>self.quota_settings(ui),
                    10=>{
                        ui.label(egui::RichText::new("Choose the pages and status widgets you need.").size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                        settings_label(ui,"Always-Visible Layout");
                        settings_card(ui,|ui|self.resident_order_settings(ui));
                        settings_label(ui,"Status Widgets");
                        settings_card(ui,|ui|{
                            setting_row(ui,"Battery","Battery level, charging status and alerts; hidden automatically on devices without a battery",&mut self.settings.show_battery);
                            ui.separator();
                            setting_row(ui,"Lyrics on Island","Show lyrics when collapsed; show cover and song title when no lyrics",&mut self.settings.island_lyrics);
                        });
                        settings_label(ui,"Feature Pages");
                        settings_card(ui,|ui|{
                            let descriptions=["Playback controls and synced lyrics","Pomodoro and break timer","Water log and reminders","Temporary file shelf","Task list and due reminders","Mail summaries and verification codes","Agent remaining quota and account switching","Monthly calendar, mail invites and work schedule"];
                            ui.label(egui::RichText::new("Drag the handle on the left to reorder; changes apply to the island on release.").size(11.5).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            #[derive(Clone)]
                            struct PageDrag(usize);
                            let mut movement=None;
                            let row_w=ui.available_width();
                            for (index,page) in self.settings.page_order.into_iter().enumerate() {
                                if index>0 {ui.separator();}
                                let mut visible=self.settings.visible_pages[page];
                                let can_change=!visible || self.settings.visible_pages.iter().filter(|enabled|**enabled).count()>1;
                                let row=ui.push_id(("page-order",page),|ui|ui.horizontal(|ui|{
                                    ui.set_max_width(row_w);
                                    let (rect,handle)=ui.allocate_exact_size(egui::vec2(20.0,40.0),egui::Sense::drag());
                                    for x in [-3.0,3.0] {for y in [-6.0,0.0,6.0] {
                                        ui.painter().circle_filled(rect.center()+egui::vec2(x,y),1.4,settings_color(ctx,Color32::from_gray(155),Color32::from_gray(100)));
                                    }}
                                    handle.on_hover_cursor(egui::CursorIcon::Grab).dnd_set_drag_payload(PageDrag(page));
                                    ui.add_enabled_ui(can_change,|ui|{setting_row(ui,PAGE_NAMES[page],descriptions[page],&mut visible);});
                                })).inner.response;
                                if let Some(payload)=row.dnd_hover_payload::<PageDrag>() {
                                    if payload.0!=page {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                        let y=if self.settings.page_order.iter().position(|p|*p==payload.0).unwrap()<index {row.rect.bottom()}else{row.rect.top()};
                                        ui.painter().line_segment([egui::pos2(row.rect.left(),y),egui::pos2(row.rect.right(),y)],egui::Stroke::new(2.0,Color32::from_rgb(70,150,255)));
                                    }
                                }
                                if let Some(payload)=row.dnd_release_payload::<PageDrag>() {movement=Some((payload.0,index));}
                                if visible!=self.settings.visible_pages[page] {self.set_page_visible(page,visible);}
                            }
                            if let Some((page,to))=movement {
                                let from=self.settings.page_order.iter().position(|p|*p==page).unwrap();
                                if from<to {self.settings.page_order[from..=to].rotate_left(1);}
                                else {self.settings.page_order[to..=from].rotate_right(1);}
                            }
                            if self.settings.visible_pages.iter().filter(|enabled|**enabled).count()==1 {
                                ui.add_space(5.0);
                                ui.label(egui::RichText::new("Keep at least one page. Turning a page off only hides it; its data and reminder settings are kept.").size(11.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            }
                        });
                    }
                    0=>{
                        settings_label(ui,"Settings Appearance");
                        settings_card(ui,|ui|{
                            use crate::settings_theme::SettingsTheme;
                            settings_control_row_right(ui,"Theme",|ui|{
                                if settings_segmented(ui,&mut self.settings.settings_theme,&[(SettingsTheme::System,"System"),(SettingsTheme::Light,"Light"),(SettingsTheme::Dark,"Dark")],52.0) {ctx.request_repaint();}
                            });
                            settings_hint(ui,"Only affects the Settings window; the island’s look stays the same.");
                        });
                        settings_label(ui,"Startup");
                        settings_card(ui,|ui|{
                            let mut a=autostart;
                            setting_row(ui,"Launch at startup","Open Daozai automatically when you sign in to Windows",&mut a);
                            if a!=autostart{self.pending.push(Cmd::ToggleAutostart);}
                        });
                        settings_label(ui,"Island Appearance");
                        settings_card(ui,|ui|{
                            use crate::model::{self as model,Skin};
                            let skins:Vec<(Skin,&str)>=Skin::ALL.iter().map(|s|(*s,s.label())).collect();
                            settings_control_row_right(ui,"Skin",|ui|{settings_segmented(ui,&mut self.settings.skin,&skins,64.0);});
                            settings_hint(ui,self.settings.skin.description());
                            ui.separator();
                            let shapes:Vec<(model::IslandShape,&str)>=model::IslandShape::ALL.iter().map(|s|(*s,s.label())).collect();
                            settings_control_row_right(ui,"Shape",|ui|{settings_segmented(ui,&mut self.settings.island_shape,&shapes,64.0);});
                            settings_hint(ui,self.settings.island_shape.description());
                            if self.settings.skin.is_glass() {
                                ui.separator();
                                let mut blur=self.settings.glass_blur;
                                setting_row(ui,"Background Blur","Real-time blur from the Windows compositor, matched exactly to the island shape (requires system 'Transparency effects'); when off, uses a smoky translucent fill",&mut blur);
                                self.settings.glass_blur=blur;
                            }
                            ui.separator();
                            settings_control_row(ui,"Size",56.0,|ui|{
                                ui.horizontal(|ui|{
                                    ui.spacing_mut().slider_width=180.0;
                                    island_size_slider(ui,&mut self.settings.island_size);
                                    if ui.add_enabled(self.settings.island_size!=100,egui::Button::new("Reset")).clicked(){self.settings.island_size=100;}
                                }).response
                            });
                            settings_hint(ui,"Text, icons and the mascot are redrawn at actual pixels, staying sharp when enlarged; affects only the island, not the Settings window.");
                        });
                        settings_label(ui,"Island Display");
                        settings_card(ui,|ui|{
                            let selected=self.displays.iter().find(|d|d.id==self.settings.display_id);
                            let label=if self.settings.display_id.is_empty(){"Follow Primary Display"}else{selected.map_or("Selected display not connected",|d|d.label.as_str())};
                            settings_control_row(ui,"Display",56.0,|ui|{
                                egui::ComboBox::from_id_salt("island-display").selected_text(label).width(ui.available_width().min(300.0)).truncate().show_ui(ui,|ui|{
                                    ui.selectable_value(&mut self.settings.display_id,String::new(),"Follow Primary Display");
                                    for display in &self.displays {ui.selectable_value(&mut self.settings.display_id,display.id.clone(),&display.label);}
                                }).response
                            });
                            let edge=self.settings.dock.label();
                            ui.label(egui::RichText::new(if !self.settings.display_id.is_empty()&&selected.is_none(){"Temporarily shown on the primary display; returns automatically when the selected display reconnects.".to_string()}else{format!("Shown on the selected display{edge}; falls back to the primary display while disconnected.")}).size(11.0).color(ui.visuals().weak_text_color()));
                            ui.add_space(4.0);
                            // 停靠边缘：顶部为横向；左右为竖向排列，展开面板从边缘滑出。
                            let docks:Vec<(crate::model::Dock,&str)>=crate::model::Dock::ALL.iter().map(|d|(*d,d.label())).collect();
                            settings_control_row_right(ui,"Dock",|ui|{
                                // 换边后偏移的含义变了（水平 ↔ 上下），回到居中。
                                if settings_segmented(ui,&mut self.settings.dock,&docks,64.0) {self.settings.island_offset=0;}
                            });
                            if self.settings.dock.is_side() {
                                ui.label(egui::RichText::new("When collapsed, time, battery, playback, Focus and AI Quota are stacked vertically; the expanded panel slides out from the screen edge. The mascot and island lyrics show only when docked at the top.").size(11.0).color(ui.visuals().weak_text_color()));
                            }
                            ui.add_space(4.0);
                            let side=self.settings.dock.is_side();
                            settings_control_row(ui,if side{"Vertical Position"}else{"Horizontal Position"},56.0,|ui|{
                                ui.horizontal(|ui|{
                                    let limit=crate::model::ISLAND_OFFSET_MAX;
                                    let offset=&mut self.settings.island_offset;
                                    let (less,more,less_tip,more_tip)=if side{("↑","↓","Nudge up","Nudge down")}else{("←","→","Nudge left","Nudge right")};
                                    if ui.add_enabled(*offset>-limit,egui::Button::new(less).min_size(egui::vec2(26.0,22.0))).on_hover_text(less_tip).clicked(){*offset=(*offset-5).max(-limit);}
                                    ui.spacing_mut().slider_width=150.0;
                                    ui.add(egui::Slider::new(&mut *offset,-limit..=limit).show_value(false));
                                    if ui.add_enabled(*offset<limit,egui::Button::new(more).min_size(egui::vec2(26.0,22.0))).on_hover_text(more_tip).clicked(){*offset=(*offset+5).min(limit);}
                                    if ui.add_enabled(*offset!=0,egui::Button::new("Re-center")).clicked(){*offset=0;}
                                }).response
                            });
                            let offset=self.settings.island_offset;
                            let (less,more)=if side{("Up","Down")}else{("Left","Right")};
                            let position=match offset {0=>"Centered".to_string(),o if o<0=>format!("{less} {}%",-o),o=>format!("{more} {o}%")};
                            ui.label(egui::RichText::new(format!("Current: {position}. The expanded panel and notifications always stay on screen; relative position is kept after changing display or size.")).size(11.0).color(ui.visuals().weak_text_color()));
                            ui.separator();
                            settings_control_row_right(ui,"Time Format",|ui|{settings_segmented(ui,&mut self.settings.clock_12h,&[(false,"24-hour"),(true,"12-hour")],72.0);});
                            settings_control_row_right(ui,"Day Period Text",|ui|{settings_segmented(ui,&mut self.settings.clock_period,&[(0,"Off"),(1,"AM / PM"),(2,"Detailed")],72.0);});
                            settings_hint(ui,"Detailed: early morning, morning, noon, afternoon, evening, night");
                            ui.separator();
                            let mut minimal=self.settings.minimal;
                            setting_row(ui,"Minimal Notch","Show a thin line when collapsed",&mut minimal);
                            if minimal!=self.settings.minimal {self.set_minimal(minimal);}
                            ui.separator();

                            let mut pet=self.settings.pet;
                            setting_row(ui,"Daozai Mascot","Stays visible when collapsed and sings along to music",&mut pet);
                            if pet!=self.settings.pet {self.set_pet(pet);}
                        });
                        self.avoidance.settings(ui,&mut self.settings);
                        settings_label(ui,"Sound & Display");
                        settings_card(ui,|ui|{
                            setting_row(ui,"Reminder Sound","Water, to-do and Focus reminders",&mut self.settings.sound);
                            ui.separator();
                            setting_row(ui,"Hide in Fullscreen","Hide during fullscreen games, videos and presentations",&mut self.settings.hide_fullscreen);
                        });
                    }
                    1=>{
                        settings_label(ui,"Lyrics");
                        settings_card(ui,|ui|{
                            setting_row(ui,"Show Lyrics When Expanded","Synced scrolling lyrics to the right of the player",&mut self.settings.lyrics_enabled);
                            self.show_lyrics=self.settings.lyrics_enabled;
                            ui.separator();
                            setting_row(ui,"Island Lyrics","Show the current line when collapsed; show cover and title when no synced lyrics",&mut self.settings.island_lyrics);
                        });
                        settings_label(ui,"Lyrics Timing");
                        settings_card(ui,|ui|{
                            let mut advance=-self.settings.lyric_delay_ms as f64/1000.0;
                            if setting_number(ui,"Time Offset",&mut advance,-5.0,5.0," s",0.1){self.settings.lyric_delay_ms=(-advance*1000.0).round()as i32;}
                            ui.add_space(3.0);
                            ui.small("Positive shows earlier, negative later; default is 0.35 s earlier.");
                        });
                    }
                    2=>{
                        settings_label(ui,"Timer Durations");
                        settings_card(ui,|ui|{
                            setting_hours(ui,"Focus",&mut self.settings.focus_minutes,1,crate::model::FOCUS_MAX_MINUTES);
                            ui.separator();
                            setting_number(ui,"Break",&mut self.settings.rest_minutes,1,60," min",1.0);
                            ui.add_space(3.0);
                            ui.small("Focus goes up to 24 h. Type 90, 1h30 or 2.5h. For open-ended sessions use the Stopwatch button on the Focus page. Changes take effect from the next phase.");
                        });
                    }
                    3=>{
                        settings_label(ui,"Water Reminder");
                        settings_card(ui,|ui|{
                            let old_remind=self.water.remind;
                            setting_row(ui,"Timed Reminders","Stop reminding once the daily goal is reached",&mut self.water.remind);
                            if self.water.remind!=old_remind{self.water.save();self.last_reminder=self.t;}
                            ui.separator();
                            if setting_number(ui,"Daily Goal",&mut self.water.goal_ml,500,6000," ml",50.0){self.water.save();}
                            setting_number(ui,"Per Drink",&mut self.settings.serving_ml,50,1000," ml",25.0);
                            setting_number(ui,"Interval",&mut self.settings.water_minutes,5,180," min",1.0);
                            settings_control_row(ui,"Active Hours",80.0,|ui|{
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                                    ui.allocate_ui_with_layout(egui::vec2(252.0,30.0),egui::Layout::left_to_right(egui::Align::Center),|ui|{
                                        number_stepper(ui,&mut self.settings.water_start,0,23," h",1,48.0);
                                        ui.label("to");
                                        number_stepper(ui,&mut self.settings.water_end,0,24," h",1,48.0);
                                    });
                                }).response
                            });
                        });
                    }
                    4=>{
                        settings_label(ui,"To-do List");
                        settings_card(ui,|ui|{
                            ui.label(egui::RichText::new("Add and edit tasks and set reminder times on the To-dos page.").size(13.0));
                            ui.add_space(5.0);
                            if ui.add_sized([120.0,32.0],egui::Button::new("Open To-dos")).clicked(){self.settings_open=false;self.show_page(4);}
                        });
                    }
                    5=>{
                        settings_label(ui,"Shelf");
                        settings_card(ui,|ui|{
                            ui.label(egui::RichText::new("Saves file locations; removing an item does not delete the original file.").size(13.0));
                            ui.label(egui::RichText::new(format!("{} items",self.shelf.items.len())).size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            ui.add_space(5.0);
                            if ui.add_sized([120.0,32.0],egui::Button::new("Open Shelf")).clicked(){self.settings_open=false;self.shelf_tab=0;self.show_page(3);}
                        });
                        settings_label(ui,"Clipboard");
                        settings_card(ui,|ui|{
                            setting_row(ui,"Clipboard History","View recently copied text under 'Clipboard' on the Shelf; click to copy again",&mut self.settings.clipboard_history);
                            ui.label(egui::RichText::new(format!("The latest {} entries are saved on this device and kept after reopening. Turning history off pauses collection; clearing manually keeps pinned entries. Content marked sensitive by password managers is never recorded.",crate::clipboard::MAX_RECENT)).size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            ui.add_space(5.0);
                            if ui.add_sized([120.0,32.0],egui::Button::new("Open Clipboard")).clicked(){self.settings_open=false;self.shelf_tab=1;self.show_page(3);}
                        });
                    }
                    6=>{
                        settings_label(ui,"System Alerts");
                        settings_card(ui,|ui|{
                            setting_row(ui,"System Status","Volume, charging, network and keyboard status",&mut self.settings.system_hud);
                            ui.separator();
                            setting_row(ui,"Bluetooth Notifications","Device connect/disconnect alerts; turning off stops Bluetooth monitoring",&mut self.settings.bluetooth_notifications);
                            ui.separator();
                            setting_row(ui,"All Reminders","When off, water, Focus, to-do, WeChat and QQ reminders are not shown",&mut self.settings.notifications);
                        });
                        // 总开关关闭时，微信与 QQ 两栏置灰并说明原因，避免「开了也不提醒」。
                        let master=self.settings.notifications;
                        let master_hint=|ui:&mut egui::Ui|if !master {
                            ui.label(egui::RichText::new("All Reminders is off (under 'System Alerts' above); turn it on for this to take effect").size(12.0).color(Color32::from_rgb(230,150,40)));
                        };
                        settings_label(ui,"WeChat Reminders");
                        settings_card(ui,|ui|{
                            master_hint(ui);
                            ui.add_enabled_ui(master,|ui|{
                            setting_row(ui,"WeChat New Message Alerts","Based on taskbar flashing; contacts and chat content are not read",&mut self.settings.wechat_notifications);
                            if self.settings.wechat_notifications {
                                ui.separator();
                                setting_row(ui,"Open WeChat from Tray","When WeChat is minimized to the tray, clicking the alert has the system click WeChat's tray icon; if the icon is in the overflow area, the tray menu briefly pops up",&mut self.settings.wechat_tray_open);
                            }
                            if master && !self.wechat_status.is_empty(){ui.label(egui::RichText::new(&self.wechat_status).size(12.0));}
                            });
                        });
                        settings_label(ui,"QQ & Windows Notifications");
                        settings_card(ui,|ui|{
                            master_hint(ui);
                            ui.add_enabled_ui(master,|ui|{
                            ui.label(egui::RichText::new(&self.listener_status).size(13.0));
                            if self.settings.read_notifications {
                                if ui.button(if self.settings.qq_only{"Disconnect QQ Notifications"}else{"Disconnect System Notifications"}).clicked(){self.pending.push(Cmd::DisconnectNotifications);}
                            }else{
                                ui.horizontal_wrapped(|ui|{
                                    if ui.button("Connect QQ Only").clicked(){self.pending.push(Cmd::RequestQqNotifications);}
                                    if ui.button("Connect All App Notifications").clicked(){self.pending.push(Cmd::RequestNotifications);}
                                });
                            }
                            ui.label(egui::RichText::new("QQ requires system notifications to be on; new notifications show on the island for about 4.5 s, with content hidden by default.").size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            ui.separator();
                            setting_row(ui,"Show Notification Content","May contain private messages; turn off when screen sharing",&mut self.settings.notification_preview);
                            ui.label(egui::RichText::new("Only content visible in Windows Notification Center is received; apps appear below after they send a notification.").size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            let before_apps=self.notification_apps.items.clone();
                            for (_,name,enabled) in &mut self.notification_apps.items {setting_row(ui,name,"",enabled);}
                            if before_apps!=self.notification_apps.items{self.notification_apps.save();}
                            });
                        });
                    }
                    7=>{
                        settings_label(ui,"Mail Alerts");
                        settings_card(ui,|ui|{
                            setting_row(ui,"New Mail Alerts","Show an alert when new mail arrives; island lyrics mode shows an icon on the right",&mut self.settings.mail_enabled);
                            ui.separator();
                            let sync_before=self.settings.mail_sync_operations;
                            ui.add_enabled_ui(self.mail.manage.pending.is_empty(),|ui|{
                                setting_row(ui,"Sync Mail Actions to Mailbox","When on, read, delete and restore are synced; when off, only Daozai's records are tidied",&mut self.settings.mail_sync_operations);
                            });
                            if sync_before!=self.settings.mail_sync_operations{self.mail_manage_mode_changed();}
                            ui.label(egui::RichText::new("Toggling does not retroactively sync earlier local actions.").size(11.5).color(settings_color(ctx,Color32::from_gray(195),Color32::from_gray(60))));
                            ui.separator();
                            setting_row(ui,"Highlight Verification Codes","Enlarge detected verification codes on the island; click to copy",&mut self.settings.mail_code);
                            ui.separator();
                            setting_choices(ui,"Hide Code After",&mut self.settings.mail_code_hide_seconds,[30,60,300],"s");
                            ui.label("Default 60 s. Hidden sooner if a shorter validity is stated; codes in older mail are hidden by default. Hidden does not mean expired; after revealing, it hides again after 15 s.");
                            setting_choices(ui,"Check Interval",&mut self.settings.mail_interval,[60,120,300],"s");
                            ui.label(egui::RichText::new("Mail is received in real time when push is supported, with a re-check every 10 s; the interval above applies only to mailboxes without push.").size(11.5).color(settings_color(ctx,Color32::from_gray(195),Color32::from_gray(60))));
                            ui.separator();
                            setting_choices(ui,"Meeting Lead Time",&mut self.settings.mail_ics,[5,15,30],"min");
                            ui.label(egui::RichText::new("Reminders from calendar invites in mail; shared with Schedule settings.").size(11.5).color(settings_color(ctx,Color32::from_gray(195),Color32::from_gray(60))));
                        });
                        settings_label(ui,"Added Mailboxes");
                        settings_card(ui,|ui|{
                            if self.mail.accounts.is_empty(){ui.label(egui::RichText::new("No mailboxes yet. Add one below; it connects automatically after saving.").size(12.0).color(settings_color(ctx,Color32::from_gray(195),Color32::from_gray(60))));}
                            let total=self.mail.accounts.len();
                            let(mut changed,mut remove,mut test)=(false,None,None);
                            for i in 0..total{
                                let(email,provider)=(self.mail.accounts[i].email.clone(),self.mail.accounts[i].provider);
                                let status=self.mail.status_of(&email).label();
                                let mut on=self.mail.accounts[i].enabled;
                                if i>0{ui.separator();}
                                ui.label(egui::RichText::new(&email).size(13.0).color(settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41))));
                                ui.label(egui::RichText::new(format!("{} · {}",provider.label(),status)).size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                                ui.horizontal(|ui|{
                                    if ui.add_sized([66.0,30.0],egui::Button::new("Test")).clicked(){test=Some(i);}
                                    if ui.add_sized([66.0,30.0],egui::Button::new("Delete")).clicked(){remove=Some(i);}
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new("Enabled").size(12.0).color(settings_color(ctx,Color32::from_gray(190),Color32::from_gray(65))));
                                    if switch(ui,&mut on){self.mail.accounts[i].enabled=on;changed=true;}
                                });
                            }
                            if changed{self.mail_accounts_changed();}
                            if let Some(i)=remove{
                                let email=self.mail.accounts[i].email.clone();
                                let credential=self.mail.accounts[i].credential_key();
                                self.mail.accounts.remove(i);
                                crate::mail::secrets::delete(&credential);
                                self.mail.status.remove(&email.to_ascii_lowercase());
                                self.mail.note=format!("{email}: removed; credentials deleted from Windows Credential Manager");
                                self.mail_accounts_changed();
                            }
                            if let Some(i)=test{
                                let account=self.mail.accounts[i].clone();
                                match crate::mail::secrets::load(&account.credential_key()){
                                    Ok(pass)=>{self.mail.testing=Some(account.email.clone());self.mail.note=format!("{}: testing connection…",account.email);self.pending.push(Cmd::MailTest(account,pass));}
                                    Err(e)=>self.mail.note=format!("{}: {e}",account.email),
                                }
                            }
                        });
                        settings_label(ui,"Add Mailbox");
                        settings_card(ui,|ui|{
                            settings_control_row(ui,"Mail Service",56.0,|ui|{
                                egui::ComboBox::from_id_salt("mail-provider").width(160.0).selected_text(self.mail.form.provider.label()).show_ui(ui,|ui|{
                                    for p in crate::mail::Provider::ALL{if ui.selectable_label(self.mail.form.provider==p,p.label()).clicked(){self.mail.form.provider=p;}}
                                }).response
                            });
                            ui.add_space(5.0);
                            ui.label(egui::RichText::new("Get an authorization code or app password").size(12.5).strong());
                            ui.add(egui::Label::new(egui::RichText::new(self.mail.form.provider.hint()).size(12.0).color(settings_color(ctx,Color32::from_gray(200),Color32::from_gray(55)))).wrap());
                            let (label,url)=self.mail.form.provider.setup_link();
                            if !url.is_empty() && ui.link(label).clicked(){self.pending.push(Cmd::OpenUrl(url.into()));}
                            ui.add_space(8.0);
                            if self.mail.form.provider==crate::mail::Provider::Custom {
                                settings_control_row(ui,"IMAP Host",80.0,|ui|{ui.text_edit_singleline(&mut self.mail.form.custom.host)});
                                settings_control_row(ui,"TLS Port",80.0,|ui|{ui.add(egui::DragValue::new(&mut self.mail.form.custom.port).range(1..=65535))});
                                settings_control_row(ui,"Username",80.0,|ui|{ui.add(egui::TextEdit::singleline(&mut self.mail.form.custom.username).vertical_align(egui::Align::Center).hint_text("Leave blank to use the email address"))});
                            }
                            let input_width=(ui.available_width()-56.0-48.0-2.0*ui.spacing().item_spacing.x).max(100.0);
                            settings_control_row(ui,"Email",56.0,|ui|{
                                ui.add_sized([input_width,30.0],egui::TextEdit::singleline(&mut self.mail.form.email).vertical_align(egui::Align::Center).hint_text("Enter email address"))
                            });
                            settings_control_row(ui,"Auth Code",56.0,|ui|{
                                let field=ui.add_sized([input_width,30.0],egui::TextEdit::singleline(&mut self.mail.form.password).vertical_align(egui::Align::Center).password(!self.mail.form.show).hint_text("Enter mailbox authorization code"));
                                if ui.add_sized([48.0,30.0],egui::Button::new(if self.mail.form.show{"Hide"}else{"Show"})).clicked(){self.mail.form.show=!self.mail.form.show;}
                                field
                            });
                            ui.horizontal(|ui|{
                                if ui.button("Test Connection").clicked(){
                                    let email=self.mail.form.email.trim().to_string();
                                    let target=crate::mail::Account{custom:(self.mail.form.provider==crate::mail::Provider::Custom).then(||self.mail.form.custom.clone()),email:email.clone(),provider:self.mail.form.provider,enabled:true};
                                    if !email.contains('@'){self.mail.note="Please enter an email address first".into();}
                                    else if let Err(error)=target.endpoint(){self.mail.note=error;}
                                    else if self.mail.form.password.is_empty(){self.mail.note="Please enter the authorization code first (not your mailbox login password)".into();}
                                    else{
                                        self.mail.testing=Some(email.clone());
                                        self.mail.note=format!("{email}: testing connection…");
                                        self.pending.push(Cmd::MailTest(crate::mail::Account {custom: (self.mail.form.provider==crate::mail::Provider::Custom).then(||self.mail.form.custom.clone()), email:email.clone(),provider:self.mail.form.provider,enabled:true},self.mail.form.password.clone()));
                                    }
                                }
                                if ui.button("Save & Start Monitoring").clicked(){
                                    let email=self.mail.form.email.trim().to_string();
                                    let target=crate::mail::Account{custom:(self.mail.form.provider==crate::mail::Provider::Custom).then(||self.mail.form.custom.clone()),email:email.clone(),provider:self.mail.form.provider,enabled:true};
                                    if !email.contains('@'){self.mail.note="Please enter an email address first".into();}
                                    else if let Err(error)=target.endpoint(){self.mail.note=error;}
                                    else if self.mail.form.password.is_empty(){self.mail.note="Please enter the authorization code first (not your mailbox login password)".into();}
                                    else{
                                        match crate::mail::secrets::save(&target.credential_key(),&self.mail.form.password){
                                            Ok(())=>{
                                                let account=crate::mail::Account {custom: (self.mail.form.provider==crate::mail::Provider::Custom).then(||self.mail.form.custom.clone()), email:email.clone(),provider:self.mail.form.provider,enabled:true};
                                                if let Some(slot)=self.mail.accounts.iter_mut().find(|a|a.email.eq_ignore_ascii_case(&email)){*slot=account;}else{self.mail.accounts.push(account);}
                                                self.mail.form.password.clear();
                                                self.mail.note=format!("{email}: saved, connecting…");
                                                self.mail_accounts_changed();
                                            }
                                            Err(e)=>self.mail.note=e,
                                        }
                                    }
                                }
                            });
                            ui.label(egui::RichText::new("Authorization codes are stored in Windows Credential Manager.").size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            ui.label(egui::RichText::new("For codes inside images, check your mailbox.").size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            if !self.mail.note.is_empty(){ui.add_space(3.0);ui.add(egui::Label::new(egui::RichText::new(&self.mail.note).size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75)))).wrap());}
                        });
                    }
                    8=>{
                        ui.spacing_mut().item_spacing.y=4.0;
                        // v0.5.6 日程：状态在最上面，后面才是平台、账号和提醒。
                        // 只读；服务器地址由平台决定，用户不用抄。
                        if let Some(cfg)=self.calendar.config.clone(){
                            settings_card(ui,|ui|{
                                let n=self.calendar.events.len();
                                ui.horizontal(|ui|{
                                    ui.label(egui::RichText::new(if self.calendar.connected{format!("● Connected · {n} events in next 24 h")}else if self.calendar.failures==0{"● Connecting…".into()}else{format!("● Last connection failed · {n} known events")}).size(13.0).strong().color(if self.calendar.connected{settings_color(ctx,Color32::from_rgb(120,220,160),Color32::from_rgb(24,121,64))}else{settings_color(ctx,Color32::from_rgb(240,180,110),Color32::from_rgb(145,89,12))}));
                                });
                                ui.horizontal(|ui|{
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                                        if ui.add_enabled(!self.calendar.testing,egui::Button::new("Disconnect").small()).clicked(){self.pending.push(Cmd::CalendarForget);}
                                        if ui.small_button("Sync Now").clicked(){self.calendar.note="Syncing…".into();self.calendar.note_error=false;self.pending.push(Cmd::CalendarSync);}
                                    });
                                });
                                ui.label(egui::RichText::new(format!("{} · Last synced {}",cfg.provider.label(),if self.calendar.last_sync.is_empty(){"—".to_string()}else{self.calendar.last_sync.clone()})).size(11.0).color(settings_color(ctx,Color32::from_gray(150),Color32::from_gray(105))));
                                if let Some(feed)=self.calendar.feeds.iter().find(|feed|feed.config==cfg) {
                                    let status=if feed.month.loading {"Syncing this month's events…".to_owned()}
                                        else if !feed.month.error.is_empty(){format!("Monthly sync failed: {}",feed.month.error)}
                                        else if feed.month.loaded==Some(self.calendar.view.month){format!("{} synced {} events",self.calendar.view.month.format("%Y-%m"),feed.month.events.len())}
                                        else {String::new()};
                                    if !status.is_empty(){ui.add(egui::Label::new(egui::RichText::new(status).size(12.0)).wrap());}
                                }
                                setting_row(ui,"Auto Sync","Updates events every 5 min; when off, click 'Sync Now'",&mut self.settings.calendar_auto_sync);
                                if !self.calendar.error.is_empty(){
                                    ui.add(egui::Label::new(egui::RichText::new(&self.calendar.error).size(12.0).color(settings_color(ctx,Color32::from_rgb(240,96,96),Color32::from_rgb(184,34,34)))).wrap());
                                }
                            });
                        }
                        if ui.add_sized([ui.available_width(),34.0],egui::Button::new("Open Schedule")).clicked(){self.open_calendar(None);}
                        settings_label(ui,"Choose Calendar Platform");
                        settings_card(ui,|ui|{
                            egui::Frame::new().fill(settings_color(ctx,Color32::from_rgb(29,29,31),Color32::from_rgb(236,239,244))).corner_radius(8.0).inner_margin(3).show(ui,|ui|{
                                ui.set_min_width(ui.available_width());
                                ui.spacing_mut().item_spacing.x=2.0;
                                // 六个平台分两行：飞书 / 钉钉 / 企业微信，iCloud / CalDAV / Exchange。
                                let width=((ui.available_width()-4.0)/3.0).max(80.0);
                                for row in crate::calendar::Provider::ALL.chunks(3){
                                    ui.horizontal(|ui|{
                                        for &p in row{
                                            let selected=self.calendar.form.provider==p;
                                            let button=egui::Button::new(egui::RichText::new(p.label()).size(13.0).color(if selected{settings_color(ctx,Color32::WHITE,Color32::from_rgb(30,34,41))}else{settings_color(ctx,Color32::from_gray(190),Color32::from_gray(65))}))
                                                .fill(if selected{settings_color(ctx,Color32::from_rgb(81,81,85),Color32::from_rgb(208,217,231))}else{Color32::TRANSPARENT})
                                                .stroke(egui::Stroke::NONE).corner_radius(6.0);
                                            let button=ui.add_enabled_ui(!self.calendar.testing,|ui|ui.add_sized([width,30.0],button)).inner;
                                            let button=match p{
                                                crate::calendar::Provider::ICloud=>button.on_hover_text("Apple Calendar: Apple ID + app-specific password"),
                                                crate::calendar::Provider::Custom=>button.on_hover_text("Any CalDAV calendar: Nextcloud, Synology, Fastmail, etc."),
                                                crate::calendar::Provider::Exchange=>button.on_hover_text("Company-hosted or third-party Exchange calendar (ActiveSync), read-only sync"),
                                                _=>button,
                                            };
                                            if button.clicked() && !selected{
                                                self.select_calendar_provider(p);
                                            }
                                        }
                                    });
                                }
                            });
                            ui.add_space(7.0);
                            ui.horizontal_top(|ui|{
                                ui.allocate_ui_with_layout(egui::vec2(50.0,18.0),egui::Layout::left_to_right(egui::Align::Center),|ui|{ui.set_min_size(egui::vec2(50.0,18.0));ui.label(egui::RichText::new("Server").size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));});
                                if self.calendar.form.provider.custom_server(){
                                    ui.add_sized([ui.available_width(),22.0],egui::TextEdit::singleline(&mut self.calendar.form.server).vertical_align(egui::Align::Center)
                                        .hint_text(egui::RichText::new(self.calendar.form.provider.server_hint()).color(settings_color(ctx,Color32::from_gray(155),Color32::from_gray(100)))));
                                }else{
                                    ui.add(egui::Label::new(egui::RichText::new(self.calendar.form.provider.server()).size(12.0).color(settings_color(ctx,Color32::from_gray(205),Color32::from_gray(55)))).wrap());
                                }
                            });
                        });
                        settings_label(ui,"Connect Account");
                        settings_card(ui,|ui|{
                            ui.scope(|ui|{
                                ui.visuals_mut().extreme_bg_color=settings_color(ctx,Color32::from_rgb(25,25,28),Color32::from_rgb(244,246,249));
                            let input_width=(ui.available_width()-50.0-48.0-2.0*ui.spacing().item_spacing.x).max(80.0);
                                ui.horizontal(|ui|{
                                    ui.allocate_ui_with_layout(egui::vec2(50.0,30.0),egui::Layout::left_to_right(egui::Align::Center),|ui|{ui.set_min_size(egui::vec2(50.0,30.0));ui.label(egui::RichText::new("Username").size(13.0).color(settings_color(ctx,Color32::from_gray(210),Color32::from_gray(55))));});
                                    let user=ui.add_sized([input_width,30.0],egui::TextEdit::singleline(&mut self.calendar.form.username).vertical_align(egui::Align::Center)
                                        .hint_text(egui::RichText::new("Enter username or paste account info").color(settings_color(ctx,Color32::from_gray(155),Color32::from_gray(100)))));
                                    if user.changed(){
                                        let pasted=self.calendar.form.username.clone();
                                        if let Some((u,p))=crate::calendar::parse_paste(&pasted){
                                            self.calendar.form.username=u;
                                            self.calendar.form.password=p;
                                            self.calendar.note="Username and password filled in from pasted content".into();
                                            self.calendar.note_error=false;
                                        }
                                    }
                                });
                                ui.add_space(5.0);
                                ui.horizontal(|ui|{
                                    ui.allocate_ui_with_layout(egui::vec2(50.0,30.0),egui::Layout::left_to_right(egui::Align::Center),|ui|{ui.set_min_size(egui::vec2(50.0,30.0));ui.label(egui::RichText::new("Password").size(13.0).color(settings_color(ctx,Color32::from_gray(210),Color32::from_gray(55))));});
                                    ui.add_sized([input_width,30.0],egui::TextEdit::singleline(&mut self.calendar.form.password).vertical_align(egui::Align::Center)
                                        .password(!self.calendar.form.show).hint_text(egui::RichText::new("Enter the password generated by the platform").color(settings_color(ctx,Color32::from_gray(155),Color32::from_gray(100)))));
                                    if ui.add_sized([48.0,30.0],egui::Button::new(if self.calendar.form.show{"Hide"}else{"Show"})).clicked(){self.calendar.form.show=!self.calendar.form.show;}
                                });
                            });
                            ui.add_space(7.0);
                            let guide=if self.calendar.form.guide_open{"Hide instructions"}else{"How do I get the username and password?"};
                            if ui.link(egui::RichText::new(guide).color(settings_color(ctx,Color32::from_rgb(120,185,245),Color32::from_rgb(26,100,177)))).clicked(){self.calendar.form.guide_open=!self.calendar.form.guide_open;}
                            if self.calendar.form.guide_open{
                                ui.add_space(4.0);
                                for(i,step)in self.calendar.form.provider.guide().iter().enumerate(){
                                    ui.label(egui::RichText::new(format!("{}. {}",i+1,step)).size(12.0).color(settings_color(ctx,Color32::from_gray(200),Color32::from_gray(55))));
                                }
                            }
                            ui.add_space(9.0);
                            let busy=self.calendar.testing;
                            let label=if busy{"Testing connection…"}else{"Test & Save"};
                            let btn=egui::Button::new(egui::RichText::new(label).size(13.0).strong().color(Color32::WHITE))
                                .fill(if busy{settings_color(ctx,Color32::from_rgb(70,70,76),Color32::from_rgb(213,219,227))}else{Color32::from_rgb(45,105,165)})
                                .corner_radius(7.0);
                            if ui.add_enabled_ui(!busy,|ui|ui.add_sized([130.0,32.0],btn)).inner.clicked(){
                                let user=self.calendar.form.username.trim().to_string();
                                let custom=self.calendar.form.provider.custom_server();
                                let server=crate::calendar::normalize_server(&self.calendar.form.server);
                                if custom && server.is_none(){self.calendar.note="Please enter a CalDAV server address starting with https://".into();self.calendar.note_error=true;}
                                else if user.is_empty(){self.calendar.note="Please enter a username first".into();self.calendar.note_error=true;}
                                else if self.calendar.form.password.is_empty(){self.calendar.note="Please enter a password first".into();self.calendar.note_error=true;}
                                else{
                                    let cfg=crate::calendar::Config{provider:self.calendar.form.provider,username:user,href:None,server:if custom{server}else{None}};
                                    self.calendar.testing=true;
                                    self.calendar.test_password=self.calendar.form.password.clone();
                                    self.calendar.note="Connecting to calendar…".into();
                                    self.calendar.note_error=false;
                                    self.pending.push(Cmd::CalendarTest(cfg,self.calendar.form.password.clone()));
                                }
                            }
                            ui.add_space(5.0);
                            ui.label(egui::RichText::new("Saved after a successful connection; the password is stored in Windows Credential Manager.").size(11.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            if !self.calendar.note.is_empty(){
                                ui.add_space(5.0);
                                let color=if self.calendar.note_error{settings_color(ctx,Color32::from_rgb(255,125,125),Color32::from_rgb(184,34,34))}else{settings_color(ctx,Color32::from_gray(210),Color32::from_gray(55))};
                                ui.add(egui::Label::new(egui::RichText::new(&self.calendar.note).size(12.0).color(color)).wrap());
                            }
                        });
                        settings_label(ui,"Import Calendar File");
                        settings_card(ui,|ui|{
                            ui.add(egui::Label::new(egui::RichText::new("Import events from an .ics file into local Schedule, e.g. a file from 'File → Export' in Apple's Calendar app. Recurring events are expanded for the next year; importing the same file again won't add duplicates.").size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75)))).wrap());
                            ui.add_space(5.0);
                            if ui.add_sized([150.0,30.0],egui::Button::new("Choose .ics File…")).clicked(){self.pending.push(Cmd::CalendarImport);}
                            if !self.calendar.import_note.is_empty(){
                                ui.add_space(3.0);
                                ui.add(egui::Label::new(egui::RichText::new(&self.calendar.import_note).size(12.0).color(settings_color(ctx,Color32::from_gray(210),Color32::from_gray(55)))).wrap());
                            }
                        });
                        settings_label(ui,"Meeting Reminders");
                        settings_card(ui,|ui|{
                            ui.horizontal(|ui|{
                                ui.allocate_ui_with_layout(egui::vec2(50.0,32.0),egui::Layout::left_to_right(egui::Align::Center),|ui|{ui.set_min_size(egui::vec2(50.0,32.0));ui.label(egui::RichText::new("Lead").size(13.0));});
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                                    for minutes in [30,15,5]{let selected=self.settings.mail_ics==minutes;
                                        if ui.add_sized([62.0,32.0],egui::Button::new(format!("{minutes} min")).selected(selected)).clicked(){self.settings.mail_ics=minutes;}
                                    }
                                });
                            });
                            ui.add_space(5.0);
                            ui.label(egui::RichText::new("Shared with meeting invites from mail; each meeting is reminded only once.").size(11.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                        });
                    }
                    _=>{
                        settings_label(ui,"Daozai");
                        settings_card(ui,|ui|{
                            ui.horizontal(|ui|{
                                ui.label(egui::RichText::new(format!("Version {}",env!("CARGO_PKG_VERSION"))).size(13.0));
                                ui.add_space(8.0);
                                ui.vertical(|ui|self.updates.ui(ui));
                            });
                            ui.label(egui::RichText::new("A little desktop-top helper for music, Focus, reminders and Schedule.").size(12.0).color(settings_color(ctx,Color32::from_gray(180),Color32::from_gray(75))));
                            ui.horizontal_wrapped(|ui|{
                                if ui.button("Open Local Data Folder").clicked(){self.pending.push(Cmd::Open(crate::model::config_dir()));}
                                ui.label(egui::RichText::new("Device icons: Solar Icons · 480 Design").size(11.0).color(settings_color(ctx,Color32::from_gray(150),Color32::from_gray(105))));
                                ui.hyperlink_to("CC BY 4.0", "https://creativecommons.org/licenses/by/4.0/");
                            });
                        });
                        settings_label(ui,"Support the Author");
                        settings_card(ui,support_author);
                    }
                    }
                });
            });
            self.avoidance.dialog(ctx,&mut self.settings);
            if before!=format!("{:?}",self.settings){self.settings.save();}
            ctx.set_style(original_style);
        });
    }
}

/// 收款码只在「支持作者」卡片展开时占用贴图；收起卡片或关闭设置窗口即释放。
fn release_support_codes(ctx:&egui::Context){
    ctx.data_mut(|data|for i in 0..2usize {data.remove::<egui::TextureHandle>(egui::Id::new(("support-author-qr",i)));});
}

/// 从原图裁出二维码区域 `[x, y, 边长]` 并缩放到 `pixels` 见方。
fn support_code_image(bytes:&[u8],crop:[f32;3],pixels:u32)->egui::ColorImage {
    let image=image::load_from_memory(bytes).expect("embedded payment QR image is valid");
    let side=(crop[2] as u32).min(image.width().saturating_sub(crop[0] as u32)).min(image.height().saturating_sub(crop[1] as u32));
    let code=image.crop_imm(crop[0] as u32,crop[1] as u32,side,side).resize_exact(pixels,pixels,image::imageops::FilterType::Lanczos3).to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([pixels as usize,pixels as usize],code.as_raw())
}

#[test]
fn support_codes_are_cropped_small_and_released_when_hidden(){
    for (bytes,crop) in [(include_bytes!("../assets/support/support-wechat-original.jpg").as_slice(),[330.0,370.0,600.0]),(include_bytes!("../assets/support/support-alipay-original.jpg").as_slice(),[200.0,660.0,860.0])] {
        let image=support_code_image(bytes,crop,360);
        assert_eq!(image.size,[360,360]);
        assert!(image.pixels.len()*4<=600*1024,"每张收款码贴图应小于 0.6MB");
        // 裁剪结果应是二维码本身：同时含有大量深色与浅色像素。
        let dark=image.pixels.iter().filter(|p|p.r()<80).count() as f32/image.pixels.len() as f32;
        assert!((0.2..0.8).contains(&dark),"裁剪区域不像二维码：深色占比 {dark}");
    }
    let ctx=egui::Context::default();
    let _=ctx.run(Default::default(),|ctx|{
        let texture=ctx.load_texture("qr",egui::ColorImage::new([4,4],Color32::WHITE),Default::default());
        ctx.data_mut(|data|data.insert_temp(egui::Id::new(("support-author-qr",0usize)),texture));
    });
    assert_eq!(ctx.tex_manager().read().allocated().filter(|(_,meta)|meta.name=="qr").count(),1);
    release_support_codes(&ctx);
    let _=ctx.run(Default::default(),|_|{});
    assert_eq!(ctx.tex_manager().read().allocated().filter(|(_,meta)|meta.name=="qr").count(),0,"收起后应释放收款码贴图");
}
