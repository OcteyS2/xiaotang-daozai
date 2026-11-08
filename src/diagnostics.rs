//! 启动错误与 Rust 异常保存在本机，不自动上传账号或应用数据。

use std::path::{Path, PathBuf};

fn write_report(directory: &Path, title: &str, details: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join("last-error.log");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    std::fs::write(&path, format!(
        "Daozai {}\nSystem: {} / {}\nTime (Unix seconds): {timestamp}\n{title}\n{details}\n",
        env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH,
    ))?;
    Ok(path)
}

fn message(directory: &Path, title: &str, details: &str) -> String {
    match write_report(directory, title, details) {
        Ok(path) => format!("{title}\n\nPlease send the following log to the developer to help locate the problem:\n{}", path.display()),
        Err(error) => format!("{title}\n\n{details}\n\nCouldn’t save the log: {error}\nPlease send a screenshot instead."),
    }
}

pub fn report(title: &str, details: &str) {
    let text = message(&daozai::model::config_dir().join("diagnostics"), title, details);
    #[cfg(windows)]
    {
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe { MessageBoxW(None, PCWSTR(wide.as_ptr()), w!("Daozai"), MB_OK | MB_ICONERROR); }
    }
    #[cfg(not(windows))]
    eprintln!("{text}");
}

/// 仅记录最近一次启动停留的阶段，不记录账号或用户内容。
pub fn startup(stage:&str) {
    let directory=daozai::model::config_dir().join("diagnostics");
    if std::fs::create_dir_all(&directory).is_ok() {
        let _=std::fs::write(directory.join("last-startup.log"),format!(
            "Daozai {}\nProcess: {}\nStartup stage: {stage}\n",env!("CARGO_PKG_VERSION"),std::process::id()));
    }
}

/// 记录实际使用的图形设备，避免把电脑里的显卡列表误当作当前渲染设备。
pub fn graphics(cc:&eframe::CreationContext<'_>) {
    use eframe::glow::HasContext;
    let details=if let Some(gl)=&cc.gl {unsafe{format!("Vendor: {}\nRenderer: {}\nOpenGL: {}\n",
        gl.get_parameter_string(eframe::glow::VENDOR),gl.get_parameter_string(eframe::glow::RENDERER),gl.get_parameter_string(eframe::glow::VERSION))}}
    else if let Some(state)=&cc.wgpu_render_state {
        let info=state.adapter.get_info();format!("Renderer: {}\nGraphics backend: {:?}\nDevice type: {:?}\n",info.name,info.backend,info.device_type)
    }else{return;};
    let directory=daozai::model::config_dir().join("diagnostics");
    if std::fs::create_dir_all(&directory).is_ok(){let _=std::fs::write(directory.join("last-graphics.log"),details);}
}

pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        // 不写入 panic 的数据载荷，避免把邮件、凭据或登录信息带进日志。
        let location = info.location().map(|p| format!("{}:{}:{}", p.file(), p.line(), p.column()))
            .unwrap_or_else(|| "Unknown".into());
        report("Daozai hit an error, please restart it.", &format!(
            "Code location: {location}\nBacktrace:\n{}", std::backtrace::Backtrace::force_capture(),
        ));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("daozai-diagnostics-{}-{name}", std::process::id()))
    }

    #[test]
    fn failure_creates_a_report_and_exposes_its_location() {
        let directory = directory("report");
        let text = message(&directory, "启动失败", "OpenGL 初始化失败");
        let path = directory.join("last-error.log");
        assert!(text.contains(&path.display().to_string()));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(env!("CARGO_PKG_VERSION")));
        assert!(content.contains("OpenGL 初始化失败"));
        // 重复失败只保留最后一份，避免常驻程序无限累积日志。
        write_report(&directory, "第二次失败", "新的错误").unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("OpenGL"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn unwritable_log_keeps_the_error_visible() {
        let path = directory("blocked");
        std::fs::write(&path, b"test").unwrap();
        let text = message(&path, "启动失败", "OpenGL 初始化失败");
        assert!(text.contains("OpenGL 初始化失败"));
        assert!(text.contains("日志无法保存"));
        std::fs::remove_file(path).unwrap();
    }
}
