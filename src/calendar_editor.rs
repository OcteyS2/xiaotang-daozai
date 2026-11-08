//! 岛内本机日程表单。
use crate::{app::{App,Cmd,CALENDAR_PAGE},local_calendar::{self,Draft},paint::{self,Card,H},theme::{self,W}};
use eframe::egui::{self,Color32};

impl App {
    pub(crate) fn open_calendar_editor(&mut self,uid:Option<&str>){
        let draft=uid.and_then(|uid|self.calendar.local.iter().find(|entry|entry.event.uid==uid)).map(Draft::edit)
            .unwrap_or_else(||Draft::new(self.calendar.view.day));
        self.show_page(CALENDAR_PAGE);self.calendar.draft=Some(draft);
    }
    pub(crate) fn save_calendar_editor(&mut self){
        let Some(mut draft)=self.calendar.draft.take() else{return;};
        let result=(||{
            if !self.calendar.local_error.is_empty(){return Err(self.calendar.local_error.clone());}
            let entry=draft.entry()?;let mut entries=self.calendar.local.clone();
            if let Some(saved)=entries.iter_mut().find(|saved|saved.event.uid==entry.event.uid){*saved=entry.clone();}else{entries.push(entry.clone());}
            local_calendar::save(&entries)?;
            self.alerts.retain(|alert|!alert.meeting.as_ref().is_some_and(|(event,_)|event.uid==entry.event.uid));
            if self.calendar.local.iter().any(|old|old.event.uid==entry.event.uid && (old.event.start!=entry.event.start || old.reminder!=entry.reminder)) {
                self.mail.fired.remove(&crate::calendar::dedupe_key(&entry.event));
            }
            self.calendar.local=entries;
            self.open_calendar(Some(entry.event));
            Ok::<_,String>(())
        })();
        if let Err(error)=result{draft.error=error;self.calendar.draft=Some(draft);}
    }
    /// 导入 .ics 文件到本机日程：过去 30 天到未来一年，重复日程展开；同一日程再次导入会跳过。
    pub fn import_ics(&mut self,files:&[(String,String)])->String{
        if !self.calendar.local_error.is_empty(){return self.calendar.local_error.clone();}
        let now=chrono::Utc::now();
        let (lo,hi)=(now-chrono::Duration::days(30),now+chrono::Duration::days(366));
        let mut entries=self.calendar.local.clone();
        let mut known:std::collections::HashSet<_>=entries.iter().map(|entry|entry.event.uid.clone()).collect();
        let (mut added,mut skipped,mut errors)=(0usize,0usize,Vec::new());
        for (name,text) in files {
            match crate::calendar::events_from_ics(text,lo,hi) {
                Ok(events)=>for mut event in events {
                    if entries.len()>=5000 {skipped+=1;continue;}
                    event.uid=format!("ics-{}-{}",event.uid,event.start);
                    if !known.insert(event.uid.clone()){skipped+=1;continue;}
                    let end=event.end.filter(|end|*end>event.start).unwrap_or(event.start+if event.all_day{86400}else{3600});
                    event.end=Some(end);
                    let reminder=(!event.all_day).then_some(15);
                    entries.push(local_calendar::Entry{event,reminder});added+=1;
                },
                Err(error)=>errors.push(format!("{name}: {error}")),
            }
        }
        if added>0 {
            if let Err(error)=local_calendar::save(&entries){return error;}
            self.calendar.local=entries;
        }
        let mut note=if added>0{format!("Imported {added} events")}else if errors.is_empty(){"No events to import in this file (only the past 30 days to one year ahead are imported)".into()}else{String::new()};
        if skipped>0{note.push_str(&format!(", {skipped} skipped (already exist)"));}
        if !errors.is_empty(){if !note.is_empty(){note.push('；');}note.push_str(&errors.join("; "));}
        self.calendar.import_note=note.clone();
        note
    }
    fn delete_calendar_editor(&mut self){
        let Some(draft)=self.calendar.draft.as_mut() else{return;};
        let Some(uid)=draft.uid.as_ref() else{return;};
        let entries:Vec<_>=self.calendar.local.iter().filter(|entry|entry.event.uid!=*uid).cloned().collect();
        match local_calendar::save(&entries){
            Ok(())=>{
                self.alerts.retain(|alert|!alert.meeting.as_ref().is_some_and(|(event,_)|event.uid==*uid));
                if self.notice.as_ref().is_some_and(|alert|alert.meeting.as_ref().is_some_and(|(event,_)|event.uid==*uid)){self.notice=None;}
                self.calendar.local=entries;self.calendar.draft=None;self.calendar.view.focus=None;
            }
            Err(error)=>draft.error=error,
        }
    }
    pub(crate) fn draw_calendar_editor(&mut self,c:&Card){
        let Some(d)=self.calendar.draft.as_mut() else{return;};
        let accent=Color32::from_rgb(82,162,255);
        c.centered_text(43.0,20.0,if d.uid.is_some(){"Edit Event"}else{"New Event"},13.0,W::Semibold,theme::WHITE,H::Left);
        c.centered_text(370.0,20.0,"Saved on this device only",9.0,W::Regular,theme::GRAY,H::Right);
        for (label,y) in [("Name",49.0),("Start",82.0),("End",114.0),("Reminder",147.0)]{c.centered_text(43.0,y,label,10.0,W::Regular,theme::GRAY,H::Left);}
        for (x,y,w) in [(86.0,38.0,284.0),(86.0,71.0,120.0),(86.0,103.0,120.0)]{c.fill(x,y,w,23.0,6.0,theme::CARD);}
        if !d.all_day {for y in [71.0,103.0]{c.fill(214.0,y,66.0,23.0,6.0,theme::CARD);}}
        let (all_day,b)=paint::capsule(c,294.0,72.0,76.0,21.0,"calendar-editor-all-day",if d.all_day{Color32::from_rgb(28,65,100)}else{theme::BTN},theme::BTN_HOVER);
        b.centered_text(332.0,82.5,if d.all_day{"✓ All day"}else{"All day"},10.0,W::Regular,theme::WHITE,H::Center);
        if all_day.clicked(){d.all_day=!d.all_day;d.error.clear();}
        if d.all_day {c.centered_text(214.0,114.0,"Includes end day",8.5,W::Regular,theme::GRAY,H::Left);}
        let options=if d.all_day{vec![(None,"None"),(Some(0),"09:00 that day")]}else{vec![(None,"None"),(Some(0),"On time"),(Some(5),"5m before"),(Some(15),"15m before"),(Some(30),"30m before")]};
        let width=if d.all_day{126.0}else{54.0};
        for (i,(value,label)) in options.into_iter().enumerate(){
            let x=86.0+i as f32*(width+3.0);let on=if d.all_day{d.reminder.is_some()==value.is_some()}else{d.reminder==value};
            let (r,b)=paint::capsule(c,x,136.0,width,21.0,&format!("calendar-editor-reminder-{i}"),if on{Color32::from_rgb(28,65,100)}else{theme::BTN},theme::BTN_HOVER);
            b.centered_text(x+width/2.0,146.5,label,8.4,W::Regular,theme::WHITE,H::Center);
            if r.clicked(){d.reminder=value;}
        }
        let message=if d.error.is_empty(){"Date YYYY-MM-DD · Time HH:MM"}else{&d.error};
        c.centered_text(86.0,172.0,&c.fit(message,9.0,W::Regular,284.0),9.0,W::Regular,if d.error.is_empty(){theme::GRAY}else{theme::RED},H::Left);
        let mut delete=false;
        if d.uid.is_some(){
            let (r,b)=paint::capsule(c,43.0,188.0,82.0,22.0,"calendar-editor-delete",theme::BTN,theme::BTN_HOVER);
            b.centered_text(84.0,199.0,if d.confirm_delete{"Confirm Delete"}else{"Delete Event"},9.5,W::Regular,theme::RED,H::Center);
            if r.clicked(){if d.confirm_delete{delete=true;}else{d.confirm_delete=true;}}
        }
        let (cancel,b)=paint::capsule(c,224.0,188.0,64.0,22.0,"calendar-editor-cancel",theme::BTN,theme::BTN_HOVER);
        b.centered_text(256.0,199.0,"Cancel",10.0,W::Regular,theme::WHITE,H::Center);
        let (save,b)=paint::capsule(c,298.0,188.0,72.0,22.0,"calendar-editor-save",accent,Color32::from_rgb(110,180,255));
        b.centered_text(334.0,199.0,"Save Locally",10.0,W::Semibold,theme::WHITE,H::Center);
        if delete{self.delete_calendar_editor();}else if cancel.clicked(){self.calendar.draft=None;}else if save.clicked(){self.save_calendar_editor();}
    }
    pub(crate) fn calendar_editor_inputs(&mut self,ui:&mut egui::Ui,origin:egui::Pos2,cmds:&mut Vec<Cmd>){
        let Some(d)=self.calendar.draft.as_mut() else{return;};
        let mut input=|text:&mut String,id:&str,x:f32,y:f32,width:f32|ui.put(egui::Rect::from_min_size(origin+egui::vec2(x,y),egui::vec2(width,19.0)),
            egui::TextEdit::singleline(text).id(egui::Id::new(id)).frame(false).margin(egui::Margin::ZERO).text_color(theme::WHITE).font(theme::font(11.0,W::Regular)));
        let name=input(&mut d.title,"calendar-local-title",92.0,41.0,272.0);
        input(&mut d.start_date,"calendar-local-start-date",92.0,74.0,108.0);
        input(&mut d.end_date,"calendar-local-end-date",92.0,106.0,108.0);
        if !d.all_day {input(&mut d.start_time,"calendar-local-start-time",220.0,74.0,54.0);input(&mut d.end_time,"calendar-local-end-time",220.0,106.0,54.0);}
        let first=!d.focused;if first{name.request_focus();d.focused=true;cmds.push(Cmd::Focus);}
        if !first && ui.input(|i|i.key_pressed(egui::Key::Enter)){self.save_calendar_editor();}
    }
}
