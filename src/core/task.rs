use color_eyre::eyre::Result;
use std::future::Future;
use tokio::task::JoinHandle;
use tracing::instrument;

#[allow(unused_variables)]
#[instrument(level = "debug", skip_all)]
pub fn new<
	F: FnOnce() -> S,
	S: AsRef<str>,
	A: Future<Output = O> + Send + 'static,
	O: Send + 'static,
>(
	name_fn: F,
	async_future: A,
) -> Result<JoinHandle<O>> {
	#[cfg(not(feature = "profile_tokio"))]
	let result = Ok(tokio::task::spawn(async_future));
	#[cfg(feature = "profile_tokio")]
	let result = tokio::task::Builder::new()
		.name(name_fn().as_ref())
		.spawn(async_future);
	result
}
