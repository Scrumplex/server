use crate::wayland::surface::CoreSurface;

use super::state::{ClientState, WaylandState};
use portable_atomic::{AtomicU32, Ordering};
use smithay::{
	delegate_compositor,
	reexports::wayland_server::{protocol::wl_surface::WlSurface, Client},
	wayland::compositor::{self, CompositorClientState, CompositorHandler, CompositorState},
};
use std::sync::Arc;
use tracing::debug;

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut CompositorState {
		&mut self.compositor_state
	}

	fn commit(&mut self, surface: &WlSurface) {
		debug!(?surface, "Surface commit");
		let mut count = 0;
		let core_surface = compositor::with_states(surface, |data| {
			let count_new = data
				.data_map
				.insert_if_missing_threadsafe(|| AtomicU32::new(0));
			if !count_new {
				count = data
					.data_map
					.get::<AtomicU32>()
					.unwrap()
					.fetch_add(1, Ordering::Relaxed);
			}

			data.data_map.get::<Arc<CoreSurface>>().cloned()
		});
		if let Some(core_surface) = core_surface {
			core_surface.commit(count);
		}
	}

	fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
		&client.get_data::<ClientState>().unwrap().compositor_state
	}
}

delegate_compositor!(WaylandState);
