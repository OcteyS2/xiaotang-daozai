//! 岛内日程：左侧月历，右侧按时间排列邮件邀请与工作日程。
use crate::{app::{App, Cmd, Env, CALENDAR_PAGE}, calendar, mail, paint::{self,Card,H}, theme::{self,W}};
use chrono::{Datelike, NaiveDate};
use eframe::egui::{self,Color32};

pub(crate) struct State {
    pub month: NaiveDate,
    pub day: NaiveDate,
    pub loaded: Option<NaiveDate>,
    pub events: Vec<mail::Meeting>,
    pub loading: bool,
    pub error: String,
    pub focus: Option<String>,
    pub scroll_to_focus: bool,
    pub first: usize,
    pub show_ended: bool,
}
impl Default for State {
    fn default() -> Self {
        let day=chrono::Local::now().date_naive();
        Self { month:calendar::month_start(day), day, loaded:None, events:vec![], loading:false,
            error:String::new(), focus:None, scroll_to_focus:false, first:0, show_ended:false }
    }
}

fn event_day(event:&mail::Meeting)->Option<NaiveDate> {
    chrono::DateTime::from_timestamp(event.start,0).map(|at|at.with_timezone(&chrono::Local).date_naive())
}

fn event_days(event:&mail::Meeting)->Option<(NaiveDate,NaiveDate)> {
    let start=event_day(event)?;
    let last=event.end.and_then(|end|chrono::DateTime::from_timestamp(end-1,0)).map(|at|at.with_timezone(&chrono::Local).date_naive()).unwrap_or(start);
    Some((start,last))
}

fn occurs_on(event:&mail::Meeting,day:NaiveDate)->bool {
    let Some((start,last))=event_days(event) else{return false;};
    start<=day && day<=last
}

#[derive(Clone,Copy,PartialEq,Eq)]
pub(crate) enum Source<'a> { Calendar(Option<calendar::Provider>), Mail(&'a str), Local }
impl Source<'_> {
    fn icon_key(self)->&'static str {
        match self {Self::Calendar(Some(provider))=>provider.key(),Self::Calendar(None)=>"calendar",Self::Mail(_)=>"mail",Self::Local=>"local"}
    }
    fn label(self)->String {
        match self {Self::Calendar(provider)=>provider.map(|p|p.label()).unwrap_or("Work Calendar").to_string(),Self::Mail(account)=>format!("Mail · {account}"),Self::Local=>"Local Calendar".into()}
    }
    fn color(self)->Color32 {
        match self {
            Self::Calendar(Some(calendar::Provider::WeCom))=>Color32::from_rgb(105,208,157),
            Self::Calendar(Some(calendar::Provider::DingTalk))=>Color32::from_rgb(77,192,242),
            Self::Calendar(Some(calendar::Provider::ICloud))=>Color32::from_rgb(255,105,97),
            Self::Calendar(_)=>Color32::from_rgb(112,162,255),
            Self::Mail(_)=>Color32::from_rgb(199,168,255),
            Self::Local=>Color32::from_rgb(105,208,157),
        }
    }
}

pub(crate) fn marker_colors(sources:&[Source<'_>])->Vec<Color32> {
    let mut colors=Vec::new();
    for source in sources {
        let color=source.color();
        if !colors.contains(&color){colors.push(color);}
        if colors.len()==3{break;}
    }
    colors
}

pub(crate) struct Entry<'a> {pub event:&'a mail::Meeting,pub sources:Vec<Source<'a>>}
fn entry_key(event:&mail::Meeting)->String {
    if event.uid.is_empty(){format!("{}|{}",event.start,event.title.trim())}else{calendar::dedupe_key(event)}
}
pub(crate) fn has_ended(event:&mail::Meeting,now:i64)->bool {
    let end=event.end.or_else(||event.all_day.then(||event_day(event)?.succ_opt()?.and_hms_opt(0,0,0)?.and_local_timezone(chrono::Local).earliest().map(|at|at.timestamp())).flatten());
    end.unwrap_or(event.start.saturating_add(60))<=now
}

fn calendar_now(env:&Env)->i64{
    chrono::NaiveDateTime::parse_from_str(&format!("{} {}",env.today,env.clock),"%Y-%m-%d %H:%M:%S").ok()
            .and_then(|at|at.and_local_timezone(chrono::Local).earliest()).map(|at|at.timestamp()).unwrap_or_else(||chrono::Local::now().timestamp())
}

fn featured_index(events:&[Entry<'_>],focus:Option<&str>,now:i64)->Option<usize> {
    let eligible=|event:&mail::Meeting|!event.all_day && !has_ended(event,now);
    events.iter().position(|entry|eligible(entry.event) && focus==Some(entry_key(entry.event).as_str()))
        .or_else(||events.iter().enumerate().filter(|(_,entry)|eligible(entry.event)).min_by_key(|(_,entry)|(entry.event.start>now,entry.event.start)).map(|(i,_)|i))
}

fn event_status(event:&mail::Meeting,now:i64,today:NaiveDate)->String {
    let left=event.start-now;
    if left>0 {
        if event_day(event)!=Some(today){"Upcoming".into()}
        else if left<60{"Starting soon".into()}
        else if left<3600{format!("in {} min",(left+59)/60)}
        else{format!("in {} h",(left+3599)/3600)}
    }else if event.end.is_some_and(|end|end<=now){"Ended".into()}
    else if event.end.is_some(){"In progress".into()}
    else{"Started".into()}
}
pub(crate) fn source_icon(c:&Card,source:Source<'_>,x:f32,y:f32,size:f32) {
    let rect=c.rect(x,y,size,size);
    let png:Option<&[u8]>=match source {
        Source::Calendar(Some(calendar::Provider::DingTalk))=>Some(include_bytes!("../assets/calendar/dingtalk.png")),
        _=>None,
    };
    if let Some(bytes)=png {
        let ctx=c.ui.ctx();
        let key=egui::Id::new(("calendar-source-logo",source.icon_key()));
        let texture=ctx.data(|data|data.get_temp::<egui::TextureHandle>(key)).unwrap_or_else(||{
            let mut image=image::load_from_memory(bytes).expect("Built-in source icon should be a valid image").into_rgba8();
            if matches!(source,Source::Calendar(Some(calendar::Provider::DingTalk))) {
                // 保留白色品牌标记，由统一图标容器绘制蓝色背景和圆角。
                for pixel in image.pixels_mut(){pixel.0=[255,255,255,(pixel[0] as u16*pixel[3] as u16/255) as u8];}
            }
            let image=egui::ColorImage::from_rgba_unmultiplied([image.width() as usize,image.height() as usize],image.as_raw());
            let texture=ctx.load_texture(source.icon_key(),image,egui::TextureOptions::LINEAR);
            ctx.data_mut(|data|data.insert_temp(key,texture.clone()));
            texture
        });
        let ding=matches!(source,Source::Calendar(Some(calendar::Provider::DingTalk)));
        c.fill(x,y,size,size,size*5.0/24.0,if ding{Color32::from_rgb(0,105,255)}else{Color32::WHITE});
        c.painter.image(texture.id(),rect.shrink(size*0.1),egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);
        return;
    }
    let icon=match source {
        Source::Calendar(Some(calendar::Provider::WeCom))=>egui::include_image!("../assets/calendar/wecom.svg"),
        Source::Mail(_)=>egui::include_image!("../assets/calendar/mail.svg"),
        Source::Calendar(Some(calendar::Provider::Feishu))=>egui::include_image!("../assets/calendar/feishu.svg"),
        _=>egui::include_image!("../assets/settings-icons/calendar-days.svg"),
    };
    let egui::ImageSource::Bytes{uri,bytes}=icon else{return};
    let pixels=(rect.width()*c.ui.ctx().pixels_per_point()).ceil() as u32;
    let icon=egui::Image::from_bytes(format!("{uri}/{pixels}.svg"),bytes);
    if let Ok(egui::load::TexturePoll::Ready{texture})=icon.fit_to_exact_size(rect.size()).load_for_size(c.ui.ctx(),rect.size()) {
        let tint=if matches!(source,Source::Calendar(None|Some(calendar::Provider::ICloud|calendar::Provider::Custom|calendar::Provider::Exchange))|Source::Local){source.color()}else{Color32::WHITE};
        c.painter.image(texture.id,rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),tint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn featured_calendar_excludes_ended_and_all_day_events() {
        let make=|uid:&str,start,end,all_day|mail::Meeting{uid:uid.into(),title:uid.into(),start,end:Some(end),all_day,at:String::new()};
        let events=[make("全天",0,86400,true),make("已结束",10,20,false),make("下一场",200,300,false),make("进行中",80,150,false)];
        let entries:Vec<_>=events.iter().map(|event|Entry{event,sources:vec![Source::Local]}).collect();
        assert_eq!(featured_index(&entries,None,100),Some(3));
        assert_eq!(featured_index(&entries,None,160),Some(2));
        assert_eq!(featured_index(&entries,Some(entry_key(&events[0]).as_str()),160),Some(2));
        assert_eq!(featured_index(&entries,None,400),None);
        assert!(has_ended(&events[1],20));
    }
    #[test]
    fn source_svg_rasterizes_at_notification_size_and_screen_scale() {
        for source in [Source::Mail("test"),Source::Calendar(Some(calendar::Provider::Feishu)),Source::Calendar(Some(calendar::Provider::WeCom))] {
        let ctx=egui::Context::default();egui_extras::install_image_loaders(&ctx);
        let raw=egui::RawInput{viewports:std::iter::once((egui::ViewportId::ROOT,egui::ViewportInfo{native_pixels_per_point:Some(2.0),..Default::default()})).collect(),..Default::default()};
        let output=ctx.run(raw,|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{
                let c=Card::new(ui,egui::Pos2::ZERO,ui.clip_rect());
                source_icon(&c,source,0.0,0.0,13.0);
                source_icon(&c,source,40.0,0.0,28.0);
            });
        });
        assert!(output.textures_delta.set.iter().any(|(_,delta)|delta.image.size()==[56,56]),"放大的通知图标必须按屏幕像素重新栅格化，不能拉伸小图：{:?}",output.textures_delta.set.iter().map(|(_,d)|d.image.size()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn dingtalk_logo_has_enough_pixels_for_large_notifications() {
        let image=image::load_from_memory(include_bytes!("../assets/calendar/dingtalk.png")).unwrap();
        assert!(image.width()>=256 && image.height()>=256,"通知不能放大低分辨率网站图标");
    }

    #[test]
    fn date_markers_merge_repeat_sources_and_show_at_most_three_colors() {
        let feishu=Source::Calendar(Some(calendar::Provider::Feishu));
        let mail=Source::Mail("work@example.com");
        assert_eq!(marker_colors(&[feishu,mail,Source::Mail("other@example.com"),feishu]),vec![feishu.color(),mail.color()]);
        let colors=marker_colors(&[feishu,Source::Calendar(Some(calendar::Provider::DingTalk)),Source::Calendar(Some(calendar::Provider::WeCom)),mail]);
        assert_eq!(colors.len(),3);
        assert!(colors.iter().enumerate().all(|(i,color)|!colors[..i].contains(color)));
    }
}

impl App {
    pub(crate) fn calendar_scroll_max(&self,env:&Env)->usize{
        let now=calendar_now(env);
        let events:Vec<_>=self.calendar_entries(self.calendar.view.day).into_iter()
            .filter(|entry|self.calendar.view.show_ended || !has_ended(entry.event,now)).collect();
        let featured=featured_index(&events,self.calendar.view.focus.as_deref(),now).is_some();
        events.len().saturating_sub(usize::from(featured)+4)
    }

    pub(crate) fn mail_event_ignored(&self,event:&mail::Meeting)->bool {
        self.settings.ignored_mail_events.contains(&calendar::dedupe_key(event))
    }
    pub(crate) fn toggle_mail_event(&mut self,event:&mail::Meeting) {
        let key=calendar::dedupe_key(event);
        if self.mail_event_ignored(event){self.settings.ignored_mail_events.retain(|k|k!=&key);}
        else{self.settings.ignored_mail_events.push(key);}
        self.settings.save();
    }

    pub fn open_calendar(&mut self, meeting:Option<mail::Meeting>) {
        if let Some(meeting)=meeting {
            if let Some(day)=event_day(&meeting){self.select_calendar_day(day);}
            self.calendar.view.show_ended=has_ended(&meeting,chrono::Local::now().timestamp());
            self.calendar.view.focus=Some(entry_key(&meeting));
            self.calendar.view.scroll_to_focus=true;
        }
        self.show_page(CALENDAR_PAGE);
        self.request_calendar_month(false);
    }

    pub(crate) fn select_calendar_day(&mut self, day:NaiveDate) {
        let month=calendar::month_start(day);
        if month!=self.calendar.view.month {self.calendar.view.month=month;self.calendar.view.loading=false;self.calendar.view.error.clear();}
        self.calendar.view.day=day;
        self.calendar.view.show_ended=day<chrono::Local::now().date_naive();
        self.calendar.view.focus=None;
        self.calendar.view.scroll_to_focus=false;
        self.calendar.view.first=0;
        self.request_calendar_month(false);
    }

    pub(crate) fn request_calendar_month(&mut self, force:bool) {
        let has_accounts=!self.calendar_configs().is_empty();
        let view=&mut self.calendar.view;
        if !has_accounts || !force && (view.loaded==Some(view.month) || view.loading || !view.error.is_empty()){return;}
        view.loading=true;view.error.clear();
        for feed in &mut self.calendar.feeds {feed.month.loading=true;feed.month.error.clear();}
        self.pending.push(Cmd::CalendarMonth(view.month));
    }

    pub(crate) fn calendar_work_events(&self, month:Option<NaiveDate>)->Vec<(&mail::Meeting,Option<calendar::Provider>)> {
        if self.calendar.feeds.is_empty() {
            let events=if month.is_some() && self.calendar.view.loaded==month {&self.calendar.view.events}else{&self.calendar.events};
            return events.iter().map(|event|(event,self.calendar.config.as_ref().map(|cfg|cfg.provider))).collect();
        }
        self.calendar.feeds.iter().flat_map(|feed|{
            let events=if month.is_some() && feed.month.loaded==month {&feed.month.events}else{&feed.events};
            events.iter().map(move |event|(event,Some(feed.config.provider)))
        }).collect()
    }

    pub(crate) fn calendar_entries(&self, day:NaiveDate)->Vec<Entry<'_>> {
        let work=self.calendar_work_events(Some(calendar::month_start(day))).into_iter().map(|(event,provider)|(event,Source::Calendar(provider)));
        let mail=self.mail.recent.iter().filter_map(|item|item.meeting.as_ref().filter(|event|!self.mail_event_ignored(event)).map(|event|(event,Source::Mail(item.account.as_str()))));
        let mut events:Vec<Entry<'_>>=vec![];
        let mut seen=std::collections::HashMap::<String,usize>::new();
        let local=self.calendar.local.iter().map(|entry|(&entry.event,Source::Local));
        for (event,source) in work.chain(mail).chain(local).filter(|(event,_)|occurs_on(event,day)) {
            let key=entry_key(event);
            if let Some(&index)=seen.get(&key) {
                let entry=&mut events[index];
                if !entry.sources.contains(&source){entry.sources.push(source);}
            }else{seen.insert(key,events.len());events.push(Entry{event,sources:vec![source]});}
        }
        events.sort_by(|a,b|(!a.event.all_day,a.event.start,&a.event.title,&a.event.uid).cmp(&(!b.event.all_day,b.event.start,&b.event.title,&b.event.uid)));
        events
    }

    pub(crate) fn page_calendar(&mut self,c:&Card,env:&Env) {
        let accent=Color32::from_rgb(82,162,255);
        let muted=Color32::from_rgb(142,142,150);
        let today=NaiveDate::parse_from_str(&env.today,"%Y-%m-%d").unwrap_or_else(|_|chrono::Local::now().date_naive());
        let now=calendar_now(env);
        c.fill(37.0,9.0,54.0,18.0,8.0,Color32::from_rgb(30,30,33));
        source_icon(c,Source::Calendar(None),43.0,13.0,10.0);
        c.centered_text(69.0,18.0,"Schedule",10.0,W::Semibold,theme::WHITE,H::Center);
        if c.hit(37.0,9.0,54.0,18.0,"calendar-today").on_hover_text("Back to today").clicked(){self.select_calendar_day(today);}
        let (add,b)=paint::capsule(c,101.0,9.0,46.0,18.0,"calendar-add-local",theme::BTN,theme::BTN_HOVER);
        b.centered_text(124.0,18.0,"＋ New",9.0,W::Regular,theme::WHITE,H::Center);
        if add.clicked(){self.open_calendar_editor(None);return;}
        c.seg((155.0,36.0),(155.0,208.0),0.6,Color32::from_gray(38));
        let day=self.calendar.view.day;
        let weekday=["Mon","Tue","Wed","Thu","Fri","Sat","Sun"][day.weekday().num_days_from_monday() as usize];
        c.text(40.0,42.0,weekday,9.0,W::Regular,accent,H::Left);
        c.text(40.0,61.0,&format!("{}/{}",day.month(),day.day()),14.0,W::Semibold,theme::WHITE,H::Left);
        let month=self.calendar.view.month;
        c.centered_text(45.0,80.0,"‹",14.0,W::Regular,muted,H::Center);
        c.centered_text(87.0,80.0,&format!("{}/{}",month.year(),month.month()),9.5,W::Regular,theme::GRAY,H::Center);
        c.centered_text(129.0,80.0,"›",14.0,W::Regular,muted,H::Center);
        for (x,next) in [(38.0,false),(122.0,true)] {
            if c.hit(x,72.0,14.0,16.0,("calendar-month",next)).clicked(){
                let date=if next{month.checked_add_months(chrono::Months::new(1))}else{month.checked_sub_months(chrono::Months::new(1))};
                if let Some(date)=date{self.select_calendar_day(date);}
            }
        }
        for (i,label) in ["Mo","Tu","We","Th","Fr","Sa","Su"].iter().enumerate(){c.centered_text(34.0+i as f32*17.5,98.0,label,9.0,W::Regular,muted,H::Center);}
        let work=self.calendar_work_events(Some(self.calendar.view.month)).into_iter().map(|(event,provider)|(event,Source::Calendar(provider)));
        let mail=self.mail.recent.iter().filter_map(|item|item.meeting.as_ref().filter(|event|!self.mail_event_ignored(event)).map(|event|(event,Source::Mail(item.account.as_str()))));
        let grid=calendar::month_grid(self.calendar.view.month);
        let mut dots:std::collections::HashMap<NaiveDate,Vec<Source<'_>>>=std::collections::HashMap::new();
        let local=self.calendar.local.iter().map(|entry|(&entry.event,Source::Local));
        for (event,source) in work.chain(mail).chain(local) {
            let Some((start,last))=event_days(event) else{continue;};
            if last<grid[0] || start>grid[41]{continue;}
            for date in grid.iter().filter(|date|start<=**date && **date<=last) {
                dots.entry(*date).or_default().push(source);
            }
        }
        let dots:std::collections::HashMap<_,_>=dots.into_iter().map(|(date,sources)|(date,marker_colors(&sources))).collect();
        for (i,date) in grid.iter().enumerate() {
            let x=34.0+(i%7) as f32*17.5;let y=114.0+(i/7) as f32*16.5;
            let selected=*date==self.calendar.view.day;
            let hit=c.hit(x-8.75,y-7.0,17.5,16.5,("calendar-day",date));
            if selected || hit.hovered(){c.circle(x,y-0.6,6.3,if selected{accent}else{Color32::from_gray(36)});}
            if *date==today && !selected{c.ring(x,y-0.6,6.3,0.8,accent);}
            let color=if selected{theme::WHITE}else if date.month()!=self.calendar.view.month.month(){Color32::from_gray(82)}else{Color32::from_gray(219)};
            c.centered_text(x,y,&date.day().to_string(),10.2,W::Regular,color,H::Center);
            if let Some(colors)=dots.get(date){for (i,color) in colors.iter().enumerate(){c.circle(x+(i as f32-(colors.len()-1) as f32/2.0)*2.8,y+8.6,0.9,*color);}}
            if hit.on_hover_text(date.format("%Y-%m-%d").to_string()).clicked(){self.select_calendar_day(*date);}
        }
        let day=self.calendar.view.day;
        let heading=format!("{} · {}/{}",if day==today{"Today"}else{["Mon","Tue","Wed","Thu","Fri","Sat","Sun"][day.weekday().num_days_from_monday() as usize]},day.month(),day.day());
        c.text(167.0,44.0,&heading,8.8,W::Semibold,accent,H::Left);
        let status=if self.calendar.view.loading{"Syncing".to_string()}else if !self.calendar.view.error.is_empty(){"Retry sync".to_string()}else {let at=self.calendar.feeds.iter().map(|feed|feed.last_sync.as_str()).max().unwrap_or(&self.calendar.last_sync);if at.is_empty(){"".into()}else{format!("Updated {}",at.split_whitespace().last().unwrap_or(""))}};
        c.text(376.0,44.0,&status,7.4,W::Regular,muted,H::Right);
        let sync_hint=if self.calendar.view.error.is_empty(){"Sync this month"}else{self.calendar.view.error.as_str()};
        if !self.calendar_configs().is_empty() && !self.calendar.view.loading && c.hit(326.0,33.0,52.0,15.0,"calendar-sync").on_hover_text(sync_hint).clicked(){self.request_calendar_month(true);}
        let focus=self.calendar.view.focus.clone();
        let scroll_to_focus=if c.live{std::mem::take(&mut self.calendar.view.scroll_to_focus)}else{false};
        let mut first=self.calendar.view.first;
        let all_events=self.calendar_entries(day);
        let ended_count=all_events.iter().filter(|entry|has_ended(entry.event,now)).count();
        let show_ended=self.calendar.view.show_ended;
        let mut events:Vec<_>=all_events.into_iter().filter(|entry|show_ended || !has_ended(entry.event,now)).collect();
        events.sort_by_key(|entry|(has_ended(entry.event,now),!entry.event.all_day,entry.event.start));
        let mut connect=false;let mut add_local=false;let mut toggle_ended=false;
        let featured_index=featured_index(&events,focus.as_deref(),now);
        let remaining_count=events.len()-usize::from(featured_index.is_some());
        let max=remaining_count.saturating_sub(4);
        if scroll_to_focus {first=0;}
        first=first.min(max);
        if featured_index.is_none() {
            let message=if self.calendar.view.loading && events.is_empty(){"Loading events…"}else if !self.calendar.view.error.is_empty() && events.is_empty(){"Can't load events right now"}else if ended_count>0 && events.is_empty(){"Today's events have ended"}else if events.is_empty(){"No events today"}else{"No upcoming timed events"};
            c.centered_text(271.0,76.0,message,11.0,W::Regular,theme::GRAY,H::Center);
            if events.is_empty(){
                let (button,b)=paint::capsule(c,214.0,93.0,70.0,20.0,"calendar-empty-add",theme::BTN,theme::BTN_HOVER);
                b.centered_text(249.0,103.0,"＋ Add Event",9.0,W::Regular,theme::WHITE,H::Center);
                if button.clicked(){add_local=true;}
                if self.calendar_configs().is_empty(){
                    c.centered_text(326.0,103.0,"Connect Calendar",9.0,W::Regular,muted,H::Center);
                    if c.hit(290.0,93.0,72.0,20.0,"calendar-connect").clicked(){connect=true;}
                }
            }
        }
        if ended_count>0 {
            c.centered_text(248.0,44.0,&format!("{} {ended_count} ended",if show_ended{"-"}else{"+"}),8.0,W::Regular,muted,H::Left);
            if c.hit(242.0,34.0,76.0,18.0,"calendar-toggle-ended").clicked(){toggle_ended=true;}
        }
        let mut clicked=None;let mut ignored=None;let mut edit_local=None;
        let featured=featured_index.and_then(|i|events.get(i)).into_iter().map(|entry|(entry,true));
        let remaining=events.iter().enumerate().filter(|(index,_)|Some(*index)!=featured_index).map(|(_,entry)|(entry,false)).skip(first).take(4);
        for (i,(entry,featured)) in featured.chain(remaining).enumerate() {
            let event=entry.event;
            let (y,height,cy,size,mut x,icon_size)=if featured{(52.0,68.0,93.0,14.0,168.0,16.0)}else{let y=130.0+(i-usize::from(featured_index.is_some())) as f32*18.8;(y,18.8,y+8.0,10.2,204.0,13.0)};
            let selected=focus.as_deref()==Some(entry_key(event).as_str());
            let hit=c.hit(164.0,y,214.0,height,("calendar-event",entry_key(event)));
            if selected || hit.hovered(){c.fill(164.0,y,214.0,height,5.0,Color32::from_gray(if selected{29}else{20}));}
            let time=if event.all_day{"All day".to_string()}else if event_day(event)!=Some(day){"Multi-day".to_string()}else{chrono::DateTime::from_timestamp(event.start,0).unwrap().with_timezone(&chrono::Local).format("%H:%M").to_string()};
            if featured {
                c.centered_text(167.0,68.0,&time,23.0,W::Semibold,theme::WHITE,H::Left);
                let status=event_status(event,now,today);
                let width=(c.text_w(&status,7.5,W::Semibold)+14.0).max(46.0);
                let ended=event.end.is_some_and(|end|end<=now);
                c.fill(366.0-width,60.0,width,14.0,7.0,if ended{theme::BTN}else{Color32::from_rgb(28,48,68)});
                c.centered_text(366.0-width/2.0,67.0,&status,7.5,W::Semibold,if ended{muted}else{accent},H::Center);
                if let Some(end)=event.end.and_then(|end|chrono::DateTime::from_timestamp(end,0)).map(|end|end.with_timezone(&chrono::Local)) {
                    let label=if event.all_day {format!("Until {}",(end-chrono::Duration::seconds(1)).format("%m-%d"))}else{format!("Ends {}",end.format(if end.date_naive()==day{"%H:%M"}else{"%m-%d %H:%M"}))};
                    c.centered_text(168.0,110.0,&label,8.4,W::Regular,muted,H::Left);
                }
                c.seg((167.0,121.0),(376.0,121.0),0.6,Color32::from_gray(39));
            }else{c.centered_text(169.0,cy,&time,8.7,W::Regular,if selected{accent}else{muted},H::Left);}
            let sources=entry.sources.iter().map(|source|source.label()).collect::<Vec<_>>().join(" · ");
            let mut kinds=Vec::new();
            for source in &entry.sources {
                let key=source.icon_key();
                if kinds.contains(&key){continue;}
                kinds.push(key);
                source_icon(c,*source,x,cy-icon_size/2.0,icon_size);
                x+=icon_size+if featured{5.0}else{2.0};
            }
            let can_ignore=entry.sources.iter().any(|source|matches!(source,Source::Mail(_)));
            let is_local=entry.sources.contains(&Source::Local);
            let title_right=if can_ignore || is_local{352.0}else{371.0};
            c.centered_text(x+2.0,cy,&c.fit(&event.title,size,W::Semibold,title_right-x-2.0),size,W::Semibold,if has_ended(event,now){muted}else{theme::WHITE},H::Left);
            if is_local {
                c.centered_text(365.0,cy,"Edit",7.5,W::Regular,accent,H::Center);
                if c.hit(354.0,cy-8.0,23.0,16.0,("calendar-edit-local",event.uid.as_str())).clicked(){edit_local=Some(event.uid.clone());}
            }
            if can_ignore {
                let button=c.hit(357.0,cy-8.0,18.0,16.0,("calendar-ignore-mail",entry_key(event))).on_hover_text("Ignore this mail event; restore it in the mail details");
                paint::close(c,366.0,cy,3.0,if button.hovered(){theme::WHITE}else{muted});
                if button.clicked(){ignored=Some(event.clone());}
            }
            if hit.on_hover_text(format!("{}\n{}\n{}",event.title,event.at,sources)).clicked(){clicked=Some(entry_key(event));}
        }
        if remaining_count>4 {c.text(376.0,208.0,&format!("List {}–{} / {}",first+1,(first+4).min(remaining_count),remaining_count),7.0,W::Regular,muted,H::Right);}
        self.calendar.view.first=first;
        if toggle_ended{self.calendar.view.show_ended=!show_ended;self.calendar.view.first=0;}
        if let Some(key)=clicked{self.calendar.view.focus=Some(key);}
        if let Some(uid)=edit_local{self.open_calendar_editor(Some(&uid));}
        if add_local{self.open_calendar_editor(None);}
        if let Some(event)=ignored{self.toggle_mail_event(&event);}
        if connect {self.settings_tab=8;self.open_settings();}
    }
}
