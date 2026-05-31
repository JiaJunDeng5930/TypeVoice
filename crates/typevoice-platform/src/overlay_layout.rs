use tauri::{LogicalSize, PhysicalPosition};
use typevoice_storage::{data_dir, settings};

pub fn apply_overlay_layout(w: &tauri::WebviewWindow) -> anyhow::Result<()> {
    let dir = data_dir::data_dir()?;
    let s = settings::load_settings_strict(&dir)?;
    let config = settings::resolve_overlay_config(&s);
    apply_overlay_layout_with_config(w, &config)
}

pub fn apply_overlay_layout_with_config(
    w: &tauri::WebviewWindow,
    config: &settings::OverlayConfigResolved,
) -> anyhow::Result<()> {
    w.set_size(resolved_overlay_size(config))?;
    w.set_position(resolved_overlay_position(w, config))?;
    Ok(())
}

pub fn resize_overlay(w: &tauri::WebviewWindow, width: f64, height: f64) -> anyhow::Result<()> {
    w.set_size(LogicalSize::new(
        width.clamp(360.0, 1600.0),
        height.clamp(72.0, 360.0),
    ))?;
    Ok(())
}

pub fn resolved_overlay_size(config: &settings::OverlayConfigResolved) -> LogicalSize<f64> {
    LogicalSize::new(config.width_px as f64, config.height_px as f64)
}

pub fn resolved_overlay_position(
    w: &tauri::WebviewWindow,
    config: &settings::OverlayConfigResolved,
) -> PhysicalPosition<i32> {
    let pos = settings::resolve_overlay_position(config, &overlay_work_areas(w));
    PhysicalPosition::new(pos.x.round() as i32, pos.y.round() as i32)
}

pub fn overlay_work_areas(w: &tauri::WebviewWindow) -> Vec<settings::OverlayWorkArea> {
    let mut areas = Vec::new();
    if let Some(monitor) = w
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| w.primary_monitor().ok().flatten())
    {
        push_overlay_work_area(&mut areas, &monitor);
    }
    if let Ok(monitors) = w.available_monitors() {
        for monitor in monitors {
            push_overlay_work_area(&mut areas, &monitor);
        }
    }
    areas
}

fn push_overlay_work_area(areas: &mut Vec<settings::OverlayWorkArea>, monitor: &tauri::Monitor) {
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let next = settings::OverlayWorkArea {
        x: area.position.x as f64,
        y: area.position.y as f64,
        width: area.size.width as f64,
        height: area.size.height as f64,
        scale_factor: scale,
    };
    let exists = areas.iter().any(|area| {
        area.x == next.x
            && area.y == next.y
            && area.width == next.width
            && area.height == next.height
    });
    if !exists {
        areas.push(next);
    }
}
