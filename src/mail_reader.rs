//! 邮件全文阅读窗口：完整文字、收件账号和网页链接。
use crate::app::{App,Cmd};
use eframe::egui::{self,Color32};

impl App {
    pub fn mail_reader_ui(&mut self,ctx:&egui::Context) {
        let Some(raw)=self.mail.reading.clone() else{return};
        let hidden=self.mail_code_hidden(&raw);
        let item=self.display_mail(&raw);
        let mut reveal=false;
        let focus=std::mem::take(&mut self.mail.reader_focus);
        let (mut close,mut link)=(false,None);
        let mut toggle=None;
        let mut action=None;
        let folder=if self.mail.manage.reader_trash{crate::mail_manage::Folder::Trash}else{crate::mail_manage::Folder::Inbox};
        let ready=self.mail_reader_ready(&raw,folder);
        let seen=self.mail_seen(&raw,folder)==Some(true);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("daozai-mail-reader"),
            egui::ViewportBuilder::default().with_title("Full Email · Daozai").with_inner_size([700.0,620.0]).with_min_inner_size([500.0,380.0]),|ctx,_|{
            if focus{ctx.send_viewport_cmd(egui::ViewportCommand::Focus);}
            close=ctx.input(|i|i.viewport().close_requested());
            if raw.code.is_some(){ctx.request_repaint_after(std::time::Duration::from_secs(1));}
            egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::from_rgb(25,25,27)).inner_margin(22)).show(ctx,|ui|{
                ui.spacing_mut().item_spacing.y=8.0;
                ui.add(egui::Label::new(egui::RichText::new(&item.subject).size(20.0).strong().color(Color32::WHITE)).wrap());
                egui::Grid::new("mail-reader-meta").spacing([12.0,5.0]).show(ui,|ui|{
                    for (label,text) in [("From",&item.from),("To Mailbox",&item.account),("Time",&item.time)]{
                        ui.label(egui::RichText::new(label).color(Color32::from_gray(160)));
                        ui.label(egui::RichText::new(text).color(Color32::from_gray(220)));ui.end_row();
                    }
                });
                ui.horizontal(|ui|{
                    if folder==crate::mail_manage::Folder::Trash{
                        if ui.add_enabled(ready,egui::Button::new("Restore to Inbox")).clicked(){action=Some(crate::mail_manage::Action::Move{uid:raw.uid});}
                    }else{
                        if ui.add_enabled(ready,egui::Button::new(if seen{"Mark Unread"}else{"Mark Read"})).clicked(){action=Some(crate::mail_manage::Action::Read{uid:raw.uid,seen:!seen});}
                        if ui.add_enabled(ready,egui::Button::new("Move to Trash")).clicked(){action=Some(crate::mail_manage::Action::Move{uid:raw.uid});}
                    }
                    if !self.mail.manage.message.is_empty(){ui.label(egui::RichText::new(&self.mail.manage.message).size(11.0).color(Color32::from_gray(160)));}
                });
                if let Some(event)=&item.meeting {
                    ui.horizontal_wrapped(|ui|{
                        ui.label(format!("Event · {} · {}",event.at,event.title));
                        if ui.button(if self.mail_event_ignored(event){"Restore Event"}else{"Ignore Event"}).clicked(){toggle=Some(event.clone());}
                    });
                }
                if hidden {
                    ui.label("Verification code hidden; this does not mean it has expired. Related links are also temporarily hidden.");
                    reveal=ui.button("Reveal code and content for 15 s").clicked();
                }
                ui.separator();
                ui.label(egui::RichText::new("Scroll with the mouse wheel or drag the scrollbar on the right to read the full message").size(11.5).color(Color32::from_gray(160)));
                ui.style_mut().spacing.scroll.floating=false;
                egui::ScrollArea::vertical().id_salt(("mail-reader-body",&item.account,item.uid)).auto_shrink([false,false]).show(ui,|ui|{
                    ui.add(egui::Label::new(egui::RichText::new(if item.summary.is_empty(){"(This email has no body)"}else{item.summary.as_ref()}).size(14.0).color(Color32::from_gray(235))).selectable(true).wrap());
                    if !item.links.is_empty(){
                        ui.add_space(12.0);ui.separator();ui.label(egui::RichText::new("Links in This Email").strong());
                        for target in &item.links{
                            if ui.link(&target.label).on_hover_text(&target.url).clicked(){link=Some(target.url.clone());}
                        }
                    }
                });
            });
        });
        if let Some(action)=action.filter(|_|self.mail_reader_ready(&raw,folder)){self.mail_manage_request(Some(&raw.account),folder,action);ctx.request_repaint();}
        if reveal{self.reveal_mail_code(&raw);ctx.request_repaint();}
        if let Some(event)=toggle{self.toggle_mail_event(&event);}
        if close{self.mail.reading=None;}
        if let Some(url)=link{self.pending.push(Cmd::OpenUrl(url));ctx.request_repaint();}
    }
}
