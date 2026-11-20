//! Atlas Forge desktop application.

// no console window behind the program (release builds); the self-test
// writes its report to a file as well
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod ai_ui;
mod climate_ui;
mod annotation_ui;
mod area_ui;
mod app;
mod autosave;
mod checkpoint_ui;
mod brush_ui;
mod book_ui;
mod crash;
mod curve_ui;
mod draw_ui;
mod events_ui;
mod features_ui;
mod focus_ui;
mod i18n;
mod fonts_ui;
mod furniture_ui;
mod label_ui;
mod map_detail;
mod layers_ui;
mod layout_ui;
mod lifespan_ui;
mod measure_ui;
mod names_ui;
mod pageview_ui;
mod panels;
mod plugin_ui;
mod partition_ui;
mod roads_ui;
mod query_ui;
mod reshape_ui;
mod river_ui;
mod select_ui;
mod self_test;
mod shell_ui;
mod star_map;
mod styles_ui;
mod symbol_ui;
mod view;
mod wizard_ui;
mod timeline_ui;
mod tutorial_ui;
mod diplomacy_ui;
mod toolbar_ui;
mod underlay_ui;

use eframe::egui;
use std::sync::Arc;

/// MSAA sample count shared by egui's renderer and the map pipelines.
pub const MSAA: u32 = 4;

fn main() -> eframe::Result<()> {
    // `atlas --self-test <folder>`: the core at work without a window (for packages)
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--self-test") {
        let dir = std::path::PathBuf::from(args.get(2).cloned().unwrap_or_else(|| std::env::temp_dir().join("atlas_self_test").display().to_string()));
        std::process::exit(match self_test::run(&dir) {
            Ok(report) => {
                println!("{report}");
                0
            }
            Err(e) => {
                eprintln!("自检失败：{e}");
                1
            }
        });
    }
    let open_path = args.get(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        multisampling: MSAA as u16,
        depth_buffer: 0,
        viewport: egui::ViewportBuilder::default().with_title("Atlas Forge").with_inner_size([1500.0, 900.0]).with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Atlas Forge",
        options,
        Box::new(move |cc| {
            let font = map_io::png::find_system_font().map(Arc::new);
            // the system's fonts are listed in the background, for the font pickers
            map_io::fonts::warm();
            install_fonts(&cc.egui_ctx, font.clone());
            #[cfg(debug_assertions)]
            if std::env::var_os("ATLAS_DEBUG_UI").is_some() {
                cc.egui_ctx.global_style_mut(|s| {
                    s.debug.show_interactive_widgets = true;

                });
            }
            Ok(Box::new(app::App::new(cc, font, open_path)))
        }),
    )
    .inspect_err(|e| {
        // no window to show it in: a message box, with what to try
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Atlas Forge 无法启动")
            .set_description(format!(
                "无法创建窗口或图形设备：{e}

可以尝试：更新显卡驱动；在远程桌面或虚拟机中改用本机运行；或设置环境变量 WGPU_BACKEND=dx12（或 vulkan、gl）后再启动。"
            ))
            .show();
    })
}

/// Add a CJK system font as a fallback so Chinese names render.
fn install_fonts(ctx: &egui::Context, font: Option<Arc<Vec<u8>>>) {
    let Some(bytes) = font else { return };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("cjk".into(), Arc::new(egui::FontData::from_owned((*bytes).clone())));
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().push("cjk".into());
    }
    ctx.set_fonts(fonts);
}

/// Resident memory of this process in bytes (Windows); None elsewhere.
pub fn process_memory() -> Option<usize> {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[allow(non_snake_case)]
        struct Pmc {
            cb: u32,
            PageFaultCount: u32,
            PeakWorkingSetSize: usize,
            WorkingSetSize: usize,
            QuotaPeakPagedPoolUsage: usize,
            QuotaPagedPoolUsage: usize,
            QuotaPeakNonPagedPoolUsage: usize,
            QuotaNonPagedPoolUsage: usize,
            PagefileUsage: usize,
            PeakPagefileUsage: usize,
        }
        #[link(name = "psapi")]
        extern "system" {
            fn GetProcessMemoryInfo(h: isize, c: *mut Pmc, cb: u32) -> i32;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> isize;
        }
        let mut c: Pmc = unsafe { std::mem::zeroed() };
        c.cb = std::mem::size_of::<Pmc>() as u32;
        let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
        (ok != 0).then_some(c.WorkingSetSize)
    }
    #[cfg(not(windows))]
    {
        None
    }
}
