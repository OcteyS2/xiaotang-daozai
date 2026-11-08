//! OpenGL 初始化失败时使用独立的图形后端。
//!
//! 典型场景：笔记本接扩展坞 / DisplayLink 外接屏、远程桌面、显卡驱动缺失时，窗口所在的显示设备
//! 只有 Windows 自带的 OpenGL 1.1（GDI Generic），egui_glow 报「requires OpenGL 2.0+」。
//! 这时改用 Direct3D 12 重新启动，不需要用户拔扩展坞。

/// 兼容图形后端：Direct3D 12（低功耗显卡优先）。
fn compatible(mut options:eframe::NativeOptions)->eframe::NativeOptions {
    options.renderer=eframe::Renderer::Wgpu;
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup)=&mut options.wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends=eframe::wgpu::Backends::DX12|eframe::wgpu::Backends::VULKAN;
        setup.power_preference=eframe::wgpu::PowerPreference::LowPower;
    }
    options
}

/// 验收与排障：设置环境变量 `DAOZAI_RENDERER=wgpu` 直接使用兼容图形后端，
/// 在正常电脑上也能检查 OpenGL 失败后用户会看到的画面。
fn forced_compatible()->bool {
    std::env::var("DAOZAI_RENDERER").is_ok_and(|value|value.trim().eq_ignore_ascii_case("wgpu"))
}

pub fn run(mut options:eframe::NativeOptions,mut launch:impl FnMut(eframe::NativeOptions)->eframe::Result<()>)->eframe::Result<()> {
    if forced_compatible() {return launch(compatible(options));}
    options.renderer=eframe::Renderer::Glow;
    let fallback_options=compatible(options.clone());
    let error=match launch(options) {
        Ok(())=>return Ok(()),
        Err(error @ (eframe::Error::OpenGL(_)|eframe::Error::Glutin(_)|eframe::Error::NoGlutinConfigs(..)))=>error,
        Err(error)=>return Err(error),
    };
    launch(fallback_options).map_err(|fallback|eframe::Error::AppCreation(format!(
        "OpenGL failed to initialize: {error}\nFallback graphics backend also failed: {fallback}\nPlease install the graphics driver from your PC maker and try again."
    ).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opengl_20_failure_retries_without_opengl(){
        let mut attempts=0;
        let result=run(eframe::NativeOptions::default(),|options|{
            attempts+=1;
            if attempts==1 {
                assert_eq!(options.renderer,eframe::Renderer::Glow);
                return Err(eframe::Error::OpenGL("egui_glow requires OpenGL 2.0+.".to_owned().into()));
            }
            assert_eq!(options.renderer,eframe::Renderer::Wgpu);
            let eframe::egui_wgpu::WgpuSetup::CreateNew(setup)=options.wgpu_options.wgpu_setup else{panic!("应创建兼容图形设备")};
            assert!(!setup.instance_descriptor.backends.contains(eframe::wgpu::Backends::GL));
            Ok(())
        });
        assert!(result.is_ok());assert_eq!(attempts,2);
    }

    #[test]
    fn application_errors_do_not_restart_the_application(){
        let mut attempts=0;
        assert!(run(eframe::NativeOptions::default(),|_|{
            attempts+=1;Err(eframe::Error::AppCreation("应用初始化失败".into()))
        }).is_err());
        assert_eq!(attempts,1);
    }

    #[test]
    fn successful_opengl_run_is_not_restarted(){
        let mut attempts=0;
        run(eframe::NativeOptions::default(),|options|{
            attempts+=1;assert_eq!(options.renderer,eframe::Renderer::Glow);Ok(())
        }).unwrap();
        assert_eq!(attempts,1);
    }

    #[test]
    fn both_failures_are_preserved_in_the_report(){
        let mut attempts=0;
        let error=run(eframe::NativeOptions::default(),|_|{
            attempts+=1;
            if attempts==1{Err(eframe::Error::OpenGL("OpenGL 2.0".to_owned().into()))}
            else{Err(eframe::Error::AppCreation("没有可用的图形设备".into()))}
        }).unwrap_err().to_string();
        assert!(error.contains("OpenGL 2.0") && error.contains("没有可用的图形设备"));
        assert_eq!(attempts,2);
    }
}
