//! Renderer selection: which eframe backend draws the window, and on what.
//!
//! **Windows** defaults to eframe's `wgpu` backend on DirectX 12. wgpu lists
//! WARP — Microsoft's CPU rasterizer, part of every Windows since 8 / Server
//! 2012 — as a `DeviceType::Cpu` adapter, so the GUI also opens on GPU-less
//! servers, VMs (Hyper-V, cloud), Windows Sandbox and RDP sessions without
//! hardware acceleration, where OpenGL is stuck at Microsoft's 1.1 software
//! implementation and the `glow` backend cannot create a context. A real GPU
//! is preferred whenever one is present.
//!
//! **Other platforms** keep `glow` (OpenGL): Mesa's llvmpipe already covers
//! GPU-less Linux, and skipping wgpu keeps that build small.
//!
//! Overrides (flag wins over environment variable):
//! - `--renderer glow|wgpu` / `INSEARCH_RENDERER=glow|wgpu`
//! - `--software-gpu` / `INSEARCH_SOFTWARE_GPU=1` — force the WARP adapter
//!   even when a GPU exists (support/diagnostics; implies `wgpu`).
//!
//! If the default `wgpu` path fails to start, `main` relaunches the process
//! once with `--renderer glow` before giving up.

use std::path::PathBuf;

/// Which eframe backend to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Glow,
    Wgpu,
}

impl Backend {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "glow" | "opengl" | "gl" => Some(Self::Glow),
            "wgpu" | "dx12" | "directx" => Some(Self::Wgpu),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Glow => "glow",
            Self::Wgpu => "wgpu",
        }
    }
}

/// The renderer choice for this run.
#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub backend: Backend,
    /// The user chose the backend explicitly (flag or env), so don't second-guess
    /// it with an automatic fallback.
    pub explicit: bool,
    /// Prefer the CPU (WARP) adapter over any GPU. Only consulted by the
    /// Windows wgpu path; the glow-only builds parse it but never read it.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub software: bool,
}

/// Parsed command line: renderer choice plus the optional initial search root.
#[derive(Debug)]
pub struct Args {
    pub renderer: Choice,
    pub root: Option<PathBuf>,
    /// Non-renderer arguments, preserved verbatim for a fallback relaunch.
    pub passthrough: Vec<String>,
}

/// Platform default backend.
const fn default_backend() -> Backend {
    if cfg!(windows) {
        Backend::Wgpu
    } else {
        Backend::Glow
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| {
        let v = v.trim().to_ascii_lowercase();
        !(v.is_empty() || v == "0" || v == "false" || v == "no" || v == "off")
    })
}

/// Parse `std::env::args()` (and the `INSEARCH_*` environment overrides).
pub fn parse_args() -> Args {
    parse(
        std::env::args().skip(1).collect(),
        std::env::var("INSEARCH_RENDERER").ok(),
    )
}

fn parse(argv: Vec<String>, env_renderer: Option<String>) -> Args {
    let mut backend: Option<Backend> = None;
    let mut software = env_flag("INSEARCH_SOFTWARE_GPU");
    let mut root: Option<PathBuf> = None;
    let mut passthrough = Vec::new();

    if let Some(v) = env_renderer.as_deref() {
        match Backend::parse(v) {
            Some(b) => backend = Some(b),
            None => eprintln!("InSearch: ignoring INSEARCH_RENDERER={v:?} (expected glow or wgpu)"),
        }
    }

    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        let value = if let Some(v) = arg.strip_prefix("--renderer=") {
            Some(v.to_string())
        } else if arg == "--renderer" {
            it.next()
        } else {
            None
        };
        if arg == "--renderer" || arg.starts_with("--renderer=") {
            match value.as_deref().and_then(Backend::parse) {
                Some(b) => backend = Some(b),
                None => eprintln!(
                    "InSearch: ignoring --renderer {} (expected glow or wgpu)",
                    value.as_deref().unwrap_or("")
                ),
            }
            continue;
        }
        if arg == "--software-gpu" {
            software = true;
            continue;
        }
        if root.is_none() && !arg.starts_with('-') {
            root = Some(PathBuf::from(&arg));
        }
        passthrough.push(arg);
    }

    // `--software-gpu` only means something for wgpu; let it imply the backend
    // unless the user explicitly asked for glow.
    let explicit = backend.is_some();
    let mut backend = backend.unwrap_or(default_backend());
    if software && !explicit {
        backend = Backend::Wgpu;
    }
    if backend == Backend::Glow && software {
        eprintln!("InSearch: --software-gpu has no effect with the glow renderer");
        software = false;
    }

    Args {
        renderer: Choice {
            backend,
            explicit,
            software,
        },
        root,
        passthrough,
    }
}

/// Configure `opts` for the chosen backend. Returns an error if the backend
/// isn't compiled into this build (wgpu is Windows-only).
pub fn apply(choice: &Choice, opts: &mut eframe::NativeOptions) -> Result<(), String> {
    match choice.backend {
        Backend::Glow => {
            opts.renderer = eframe::Renderer::Glow;
            Ok(())
        }
        Backend::Wgpu => apply_wgpu(choice, opts),
    }
}

#[cfg(windows)]
fn apply_wgpu(choice: &Choice, opts: &mut eframe::NativeOptions) -> Result<(), String> {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
    use eframe::wgpu;
    use std::sync::Arc;

    let mut cfg = WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(setup) = &mut cfg.wgpu_setup {
        // DirectX 12 only: it is what every supported Windows ships, and it is
        // the backend that exposes WARP. `WGPU_BACKEND` still overrides for
        // diagnostics (e.g. `WGPU_BACKEND=vulkan`).
        setup.instance_descriptor.backends =
            wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
        let software = choice.software;
        setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
            select_adapter(adapters, surface, software)
        }));
    }
    opts.renderer = eframe::Renderer::Wgpu;
    opts.wgpu_options = cfg;
    Ok(())
}

#[cfg(not(windows))]
fn apply_wgpu(_choice: &Choice, _opts: &mut eframe::NativeOptions) -> Result<(), String> {
    Err("the wgpu renderer is only built into the Windows binary; use --renderer glow".into())
}

/// Pick the adapter to render on: the best hardware GPU that can present to
/// the window, falling back to WARP (`DeviceType::Cpu`) when there is none —
/// or WARP first when software rendering was requested.
#[cfg(windows)]
fn select_adapter(
    adapters: &[eframe::wgpu::Adapter],
    surface: Option<&eframe::wgpu::Surface<'_>>,
    software: bool,
) -> Result<eframe::wgpu::Adapter, String> {
    use eframe::wgpu::DeviceType;

    let usable: Vec<&eframe::wgpu::Adapter> = adapters
        .iter()
        .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
        .collect();

    // Lower is better; ties keep wgpu's enumeration order (primary adapter first).
    fn rank(a: &eframe::wgpu::Adapter) -> u8 {
        match a.get_info().device_type {
            DeviceType::DiscreteGpu => 0,
            DeviceType::IntegratedGpu => 1,
            DeviceType::VirtualGpu => 2,
            DeviceType::Other => 3,
            DeviceType::Cpu => 4,
        }
    }

    let cpu = usable
        .iter()
        .find(|a| a.get_info().device_type == DeviceType::Cpu);
    let best = usable.iter().min_by_key(|a| rank(a));
    let pick = if software { cpu.or(best) } else { best };

    match pick {
        Some(a) => {
            let info = a.get_info();
            eprintln!(
                "InSearch: rendering with {:?} adapter \"{}\" ({:?}){}",
                info.backend,
                info.name,
                info.device_type,
                if software && info.device_type != DeviceType::Cpu {
                    " — WARP not available, software rendering not possible"
                } else {
                    ""
                }
            );
            Ok((*a).clone())
        }
        None => Err(format!(
            "no usable DirectX 12 adapter (not even WARP) among {} enumerated",
            adapters.len()
        )),
    }
}

/// Human-readable description of what the running app draws with, for the
/// About window (support: "is this WARP or my GPU?").
pub fn describe(cc: &eframe::CreationContext<'_>) -> String {
    #[cfg(windows)]
    if let Some(rs) = &cc.wgpu_render_state {
        use eframe::wgpu::{Backend as B, DeviceType};
        let info = rs.adapter.get_info();
        let backend = match info.backend {
            B::Dx12 => "DirectX 12",
            B::Vulkan => "Vulkan",
            B::Gl => "OpenGL",
            B::Metal => "Metal",
            B::BrowserWebGpu => "WebGPU",
            B::Noop => "no-op",
        };
        let note = if info.device_type == DeviceType::Cpu {
            " — software rendering, no GPU"
        } else {
            ""
        };
        return format!("{backend} · {}{note}", info.name);
    }
    if cc.gl.is_some() {
        return "OpenGL (glow)".to_string();
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_first_non_flag_and_renderer_value_is_not_a_root() {
        let a = parse(
            vec!["--renderer".into(), "glow".into(), "C:\\logs".into()],
            None,
        );
        assert_eq!(a.renderer.backend, Backend::Glow);
        assert!(a.renderer.explicit);
        assert_eq!(a.root.as_deref(), Some(std::path::Path::new("C:\\logs")));
        assert_eq!(a.passthrough, vec!["C:\\logs".to_string()]);
    }

    #[test]
    fn equals_form_and_env_override() {
        let a = parse(vec!["--renderer=wgpu".into()], Some("glow".into()));
        assert_eq!(a.renderer.backend, Backend::Wgpu, "flag beats env");
        let b = parse(vec![], Some("glow".into()));
        assert_eq!(b.renderer.backend, Backend::Glow);
        assert!(b.renderer.explicit);
    }

    #[test]
    fn software_gpu_implies_wgpu_unless_glow_requested() {
        let a = parse(vec!["--software-gpu".into()], None);
        assert_eq!(a.renderer.backend, Backend::Wgpu);
        assert!(a.renderer.software);
        assert!(!a.renderer.explicit);
        let b = parse(
            vec!["--software-gpu".into(), "--renderer".into(), "glow".into()],
            None,
        );
        assert_eq!(b.renderer.backend, Backend::Glow);
        assert!(!b.renderer.software);
    }

    #[test]
    fn unknown_renderer_falls_back_to_platform_default() {
        let a = parse(vec!["--renderer".into(), "bogus".into()], None);
        assert_eq!(a.renderer.backend, default_backend());
        assert!(!a.renderer.explicit);
        assert!(a.root.is_none());
    }
}
