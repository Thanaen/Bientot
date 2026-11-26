//! What the Wayland compositor offers the flyout, read once on a connection of its own: whether
//! it has layer-shell, and its outputs.

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};

use super::{Output, WaylandShell};
use bientot_platform::placement::Rect;

/// The compositor of `WAYLAND_DISPLAY`, or `None` when it cannot be reached.
pub(super) fn read() -> Option<WaylandShell> {
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<Outputs>(&connection).ok()?;
    let handle = queue.handle();
    let (layer_shell, output_names) = globals.contents().with_list(|list| {
        let layer_shell = list
            .iter()
            .any(|global| global.interface == "zwlr_layer_shell_v1");
        let outputs: Vec<_> = list
            .iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| (global.name, global.version))
            .collect();
        (layer_shell, outputs)
    });
    let manager: Option<ZxdgOutputManagerV1> = globals.bind(&handle, 1..=3, ()).ok();
    let mut outputs = Outputs(vec![Seen::default(); output_names.len()]);
    for (index, (name, version)) in output_names.into_iter().enumerate() {
        let output: wl_output::WlOutput =
            globals
                .registry()
                .bind(name, version.min(4), &handle, index);
        if let Some(manager) = &manager {
            manager.get_xdg_output(&output, &handle, index);
        }
    }
    // The outputs answer the binding, then `xdg_output` answers its own requests.
    queue.roundtrip(&mut outputs).ok()?;
    queue.roundtrip(&mut outputs).ok()?;
    Some(WaylandShell {
        layer_shell,
        outputs: outputs.0.iter().filter_map(Seen::output).collect(),
    })
}

#[derive(Clone, Copy, Default)]
struct Seen {
    mode: Option<(i32, i32)>,
    rotated: bool,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
}

impl Seen {
    fn output(&self) -> Option<Output> {
        let (mode_width, mode_height) = self.mode?;
        let ((x, y), (width, height)) = (self.position?, self.size?);
        Some(Output {
            logical: Rect {
                x,
                y,
                width,
                height,
            },
            physical_height: if self.rotated {
                mode_width
            } else {
                mode_height
            },
        })
    }
}

struct Outputs(Vec<Seen>);

impl Dispatch<wl_output::WlOutput, usize> for Outputs {
    fn event(
        outputs: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let seen = &mut outputs.0[*index];
        match event {
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => seen.mode = Some((width, height)),
            wl_output::Event::Geometry {
                transform: WEnum::Value(transform),
                ..
            } => {
                use wl_output::Transform;
                seen.rotated = matches!(
                    transform,
                    Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, usize> for Outputs {
    fn event(
        outputs: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let seen = &mut outputs.0[*index];
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => seen.position = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                seen.size = Some((width, height))
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Outputs: ignore ZxdgOutputManagerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Outputs {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
