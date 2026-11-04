use super::{PreparedMesh, points::PreparedPoints};
use std::{
    cell::Cell,
    sync::{Arc, LazyLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderMode {
    Auto,
    Cpu,
    Gpu,
}

impl std::str::FromStr for RenderMode {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "cpu" => Ok(Self::Cpu),
            "gpu" => Ok(Self::Gpu),
            _ => Err("renderer must be auto, cpu or gpu"),
        }
    }
}

impl RenderMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

thread_local! { static OVERRIDE: Cell<Option<RenderMode>> = const { Cell::new(None) }; }

pub fn mode() -> RenderMode {
    static MODE: LazyLock<RenderMode> = LazyLock::new(|| {
        std::env::var("MELT_PREVIEW_RENDERER")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(RenderMode::Auto)
    });
    OVERRIDE
        .with(|value| value.get())
        .unwrap_or_else(|| if cfg!(test) { RenderMode::Cpu } else { *MODE })
}

/// Diagnostic/test override scoped to a caller, without mutating process env.
pub fn with_mode<T>(mode: RenderMode, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<RenderMode>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|value| value.set(self.0));
        }
    }
    let restore = Restore(OVERRIDE.with(|value| value.replace(Some(mode))));
    let result = work();
    drop(restore);
    result
}

// Provisional policy; benchmark qualification refines this before shipping.
#[cfg(any(feature = "gpu-previews", test))]
fn gpu_candidate(mode: RenderMode, size: (usize, usize)) -> bool {
    mode == RenderMode::Gpu || (mode == RenderMode::Auto && size.0 * size.1 >= 1_000_000)
}

pub(super) fn mesh(prepared: PreparedMesh, size: (usize, usize)) -> Vec<u8> {
    let prepared = Arc::new(prepared);
    #[cfg(feature = "gpu-previews")]
    if gpu_candidate(mode(), size)
        && let Ok(rgb) = super::gpu::draw(super::gpu::Scene::Mesh(prepared.clone()), size, mode())
    {
        return rgb;
    }
    super::draw_mesh_cpu(&prepared, size)
}

pub(super) fn points(prepared: PreparedPoints, size: (usize, usize)) -> Vec<u8> {
    let prepared = Arc::new(prepared);
    #[cfg(feature = "gpu-previews")]
    if gpu_candidate(mode(), size)
        && let Ok(rgb) = super::gpu::draw(super::gpu::Scene::Points(prepared.clone()), size, mode())
    {
        return rgb;
    }
    super::points::draw_cpu(&prepared, size)
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererStatus {
    pub state: String,
    pub adapter: Option<String>,
    pub backend: Option<String>,
    pub device_type: Option<String>,
    pub hardware: bool,
    pub initialization_ms: Option<f64>,
    pub reason: Option<String>,
}

impl RendererStatus {
    pub(super) fn initial() -> Self {
        Self {
            state: "uninitialized".to_owned(),
            adapter: None,
            backend: None,
            device_type: None,
            hardware: false,
            initialization_ms: None,
            reason: None,
        }
    }
}

pub fn status() -> RendererStatus {
    #[cfg(feature = "gpu-previews")]
    {
        super::gpu::status()
    }
    #[cfg(not(feature = "gpu-previews"))]
    {
        let mut status = RendererStatus::initial();
        status.state = "not-built".to_owned();
        status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_override_avoids_gpu_work_and_is_restored() {
        assert_eq!(with_mode(RenderMode::Cpu, mode), RenderMode::Cpu);
        assert_eq!(
            with_mode(RenderMode::Gpu, || with_mode(RenderMode::Auto, mode)),
            RenderMode::Auto
        );
        assert_eq!(mode(), RenderMode::Cpu);
        assert!(!gpu_candidate(RenderMode::Cpu, super::super::LARGE_SIZE));
        assert!(!gpu_candidate(RenderMode::Auto, super::super::GRID_SIZE));
        assert!(gpu_candidate(RenderMode::Gpu, super::super::GRID_SIZE));
    }
}
