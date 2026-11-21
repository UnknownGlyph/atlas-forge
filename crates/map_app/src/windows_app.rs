//! Windows input and timing around eframe's native redraw.
use eframe::{AppCreator, EframeWinitApplication, NativeOptions, UserEvent};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, DeviceEvents, EventLoop},
    window::WindowId,
};

pub(crate) fn run(
    name: &str,
    mut options: NativeOptions,
    creator: AppCreator<'_>,
) -> eframe::Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    // Atlas uses window cursor positions for drawing and camera controls.
    // Unaccelerated device deltas also woke eframe for every mouse packet.
    event_loop.listen_device_events(DeviceEvents::Never);
    prefer_direct3d(&mut options);
    let started = std::cell::Cell::new(false);
    let create = Box::new(|cc: &eframe::CreationContext<'_>| {
        let app = creator(cc)?;
        started.set(true);
        Ok(app)
    });
    let mut app = NativeApp(eframe::create_native(name, options, create, &event_loop));
    event_loop.run_app(&mut app)?;
    if !started.get() {
        // The external loop API logs eframe's renderer initialization
        // error internally. Keep the normal startup error dialog.
        return Err(eframe::Error::AppCreation(
            "The native editor could not initialize".into(),
        ));
    }
    Ok(())
}

fn prefer_direct3d(options: &mut NativeOptions) {
    use eframe::egui_wgpu::{wgpu, WgpuSetup};

    // Honour a backend, adapter or power preference set by the user.
    if ["WGPU_BACKEND", "WGPU_ADAPTER_NAME", "WGPU_POWER_PREF"]
        .iter()
        .any(|key| std::env::var_os(key).is_some())
    {
        return;
    }
    if let WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.native_adapter_selector = Some(std::sync::Arc::new(|adapters, surface| {
            adapters
                .iter()
                .filter(|adapter| surface.is_none_or(|target| adapter.is_surface_supported(target)))
                .max_by_key(|adapter| {
                    let info = adapter.get_info();
                    let hardware = match info.device_type {
                        wgpu::DeviceType::DiscreteGpu => 3,
                        wgpu::DeviceType::IntegratedGpu => 2,
                        wgpu::DeviceType::Cpu => 0,
                        _ => 1,
                    };
                    (hardware, info.backend == wgpu::Backend::Dx12)
                })
                .cloned()
                .ok_or_else(|| {
                    "No graphics adapter compatible with the Atlas Forge window".to_owned()
                })
        }));
    }
}

struct NativeApp<'a>(EframeWinitApplication<'a>);

impl ApplicationHandler<UserEvent> for NativeApp<'_> {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.0.new_events(event_loop, cause);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.0.resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window: WindowId, event: WindowEvent) {
        let _redraw = if matches!(event, WindowEvent::RedrawRequested) {
            crate::perf_ui::span("窗口重绘", "原生窗口重绘 · ms")
        } else {
            None
        };
        self.0.window_event(event_loop, window, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.0.user_event(event_loop, event);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, device: DeviceId, event: DeviceEvent) {
        self.0.device_event(event_loop, device, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.0.about_to_wait(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.0.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.0.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.0.memory_warning(event_loop);
    }
}
