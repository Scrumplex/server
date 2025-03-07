use crate::wayland::seat::SeatData;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use smithay::{
	backend::{
		allocator::dmabuf::Dmabuf,
		egl::EGLDevice,
		renderer::{gles::GlesRenderer, ImportDma},
	},
	delegate_dmabuf, delegate_output, delegate_shm,
	output::{Mode, Output, Scale, Subpixel},
	reexports::{
		wayland_protocols::xdg::{
			decoration::zv1::server::zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
			shell::server::xdg_wm_base::XdgWmBase,
		},
		wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as DecorationMode,
		wayland_server::{
			backend::{ClientData, ClientId, DisconnectReason},
			protocol::{wl_buffer::WlBuffer, wl_data_device_manager::WlDataDeviceManager},
			Display, DisplayHandle,
		},
	},
	utils::{Size, Transform},
	wayland::{
		buffer::BufferHandler,
		compositor::{CompositorClientState, CompositorState},
		dmabuf::{
			self, DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState,
			ImportError,
		},
		shell::kde::decoration::KdeDecorationState,
		shm::{ShmHandler, ShmState},
	},
};
use std::sync::{Arc, Weak};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

#[derive(Default)]
pub struct ClientState {
	pub compositor_state: CompositorClientState,
}
impl ClientData for ClientState {
	fn initialized(&self, client_id: ClientId) {
		info!("Wayland client {:?} connected", client_id);
	}

	fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
		info!(
			"Wayland client {:?} disconnected because {:#?}",
			client_id, reason
		);
	}
}

pub struct WaylandState {
	pub weak_ref: Weak<Mutex<WaylandState>>,
	pub display: Arc<Mutex<Display<WaylandState>>>,
	pub display_handle: DisplayHandle,

	pub compositor_state: CompositorState,
	// pub xdg_activation_state: XdgActivationState,
	pub kde_decoration_state: KdeDecorationState,
	pub shm_state: ShmState,
	dmabuf_state: (DmabufState, DmabufGlobal, Option<DmabufFeedback>),
	dmabuf_tx: UnboundedSender<Dmabuf>,
	pub output: Output,
	pub seats: FxHashMap<ClientId, Arc<SeatData>>,
}

impl WaylandState {
	pub fn new(
		display: Arc<Mutex<Display<WaylandState>>>,
		display_handle: DisplayHandle,
		renderer: &GlesRenderer,
		dmabuf_tx: UnboundedSender<Dmabuf>,
	) -> Arc<Mutex<Self>> {
		let compositor_state = CompositorState::new::<Self>(&display_handle);
		// let xdg_activation_state = XdgActivationState::new::<Self, _>(&display_handle);
		let kde_decoration_state =
			KdeDecorationState::new::<Self>(&display_handle, DecorationMode::Server);
		let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
		let render_node = EGLDevice::device_for_display(renderer.egl_context().display())
			.and_then(|device| device.try_get_render_node());

		let dmabuf_default_feedback = match render_node {
			Ok(Some(node)) => {
				let dmabuf_formats = renderer.dmabuf_formats().collect::<Vec<_>>();
				let dmabuf_default_feedback =
					DmabufFeedbackBuilder::new(node.dev_id(), dmabuf_formats)
						.build()
						.unwrap();
				Some(dmabuf_default_feedback)
			}
			Ok(None) => {
				warn!("failed to query render node, dmabuf will use v3");
				None
			}
			Err(err) => {
				warn!(?err, "failed to egl device for display, dmabuf will use v3");
				None
			}
		};
		// if we failed to build dmabuf feedback we fall back to dmabuf v3
		// Note: egl on Mesa requires either v4 or wl_drm (initialized with bind_wl_display)
		let dmabuf_state = if let Some(default_feedback) = dmabuf_default_feedback {
			let mut dmabuf_state = DmabufState::new();
			let dmabuf_global = dmabuf_state.create_global_with_default_feedback::<WaylandState>(
				&display_handle,
				&default_feedback,
			);
			(dmabuf_state, dmabuf_global, Some(default_feedback))
		} else {
			let dmabuf_formats = renderer.dmabuf_formats().collect::<Vec<_>>();
			let mut dmabuf_state = DmabufState::new();
			let dmabuf_global =
				dmabuf_state.create_global::<WaylandState>(&display_handle, dmabuf_formats);
			(dmabuf_state, dmabuf_global, None)
		};

		let output = Output::new(
			"1x".to_owned(),
			smithay::output::PhysicalProperties {
				size: Size::default(),
				subpixel: Subpixel::None,
				make: "Virtual XR Display".to_owned(),
				model: "Your Headset Name Here".to_owned(),
			},
		);
		let _output_global = output.create_global::<Self>(&display_handle);
		let mode = Mode {
			size: (4096, 4096).into(),
			refresh: 60000,
		};
		output.change_current_state(
			Some(mode),
			Some(Transform::Normal),
			Some(Scale::Integer(2)),
			None,
		);
		output.set_preferred(mode);
		display_handle.create_global::<Self, WlDataDeviceManager, _>(3, ());
		display_handle.create_global::<Self, XdgWmBase, _>(5, ());
		display_handle.create_global::<Self, ZxdgDecorationManagerV1, _>(1, ());

		info!("Init Wayland compositor");

		Arc::new_cyclic(|weak| {
			Mutex::new(WaylandState {
				weak_ref: weak.clone(),
				display,
				display_handle,

				compositor_state,
				// xdg_activation_state,
				kde_decoration_state,
				shm_state,
				dmabuf_state,
				dmabuf_tx,
				output,
				seats: FxHashMap::default(),
			})
		})
	}

	pub fn new_client(&mut self, client: ClientId, dh: &DisplayHandle) {
		let seat_data = SeatData::new(dh, client.clone());
		self.seats.insert(client, seat_data);
	}
}
impl Drop for WaylandState {
	fn drop(&mut self) {
		info!("Cleanly shut down the Wayland compositor");
	}
}
impl BufferHandler for WaylandState {
	fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}
impl ShmHandler for WaylandState {
	fn shm_state(&self) -> &ShmState {
		&self.shm_state
	}
}
impl DmabufHandler for WaylandState {
	fn dmabuf_state(&mut self) -> &mut DmabufState {
		&mut self.dmabuf_state.0
	}

	fn dmabuf_imported(
		&mut self,
		_global: &DmabufGlobal,
		dmabuf: Dmabuf,
	) -> Result<(), dmabuf::ImportError> {
		self.dmabuf_tx.send(dmabuf).map_err(|_| ImportError::Failed)
	}
}
delegate_dmabuf!(WaylandState);
delegate_shm!(WaylandState);
delegate_output!(WaylandState);
