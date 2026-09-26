use std::{future::Future, pin::Pin};

use crate::Result;

/// A clonable, race-free cancellation signal for structured concurrency.
///
/// Child signals are cancelled when their parent is cancelled. Cancelling a
/// child does not affect its parent or siblings.
#[derive(Debug, Clone)]
pub struct Cancellation {
    inner: tokio_util::sync::CancellationToken,
}

impl Cancellation {
    /// Creates a new cancellation scope.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// Creates a child scope linked to this scope.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            inner: self.inner.child_token(),
        }
    }

    /// Requests cancellation for this scope and its children.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    /// Completes when cancellation is requested.
    pub async fn cancelled(&self) {
        self.inner.cancelled().await;
    }
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared signals made available to every managed component.
#[derive(Debug, Clone, Default)]
pub struct LifecycleContext {
    cancellation: Cancellation,
}

impl LifecycleContext {
    /// Creates a context rooted at `cancellation`.
    #[must_use]
    pub const fn new(cancellation: Cancellation) -> Self {
        Self { cancellation }
    }

    /// Returns the cancellation signal for graceful shutdown.
    #[must_use]
    pub const fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }

    /// Creates a child context for an owned component or task group.
    #[must_use]
    pub fn child(&self) -> Self {
        Self::new(self.cancellation.child())
    }
}

/// Observable state of a managed component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LifecycleState {
    /// The component has not been started.
    Created,
    /// The component is starting and is not yet ready.
    Starting,
    /// The component is ready to serve work.
    Running,
    /// Graceful shutdown is in progress.
    Stopping,
    /// The component has stopped.
    Stopped,
    /// The component terminated because of an error.
    Failed,
}

/// The boxed future returned by object-safe lifecycle operations.
pub type BoxLifecycleFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// Explicit ownership contract for a framework-managed component.
///
/// Implementations must not detach background work. `shutdown` should request
/// termination, drain owned work, and return only after resources are released.
pub trait Lifecycle: Send {
    /// Returns the component's current state.
    fn state(&self) -> LifecycleState;

    /// Starts the component and waits until it is ready or startup fails.
    fn start<'a>(&'a mut self, context: &'a LifecycleContext) -> BoxLifecycleFuture<'a>;

    /// Gracefully stops the component and waits for owned work to finish.
    fn shutdown<'a>(&'a mut self, context: &'a LifecycleContext) -> BoxLifecycleFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_propagates_to_children_but_not_parents() {
        let parent = Cancellation::new();
        let child = parent.child();
        let sibling = parent.child();

        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
        assert!(!sibling.is_cancelled());

        parent.cancel();
        assert!(sibling.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_waiter_completes() {
        let cancellation = Cancellation::new();
        cancellation.cancel();

        cancellation.cancelled().await;
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn lifecycle_is_object_safe() {
        fn accepts_lifecycle(_: &mut dyn Lifecycle) {}

        struct Component;

        impl Lifecycle for Component {
            fn state(&self) -> LifecycleState {
                LifecycleState::Created
            }

            fn start<'a>(&'a mut self, _: &'a LifecycleContext) -> BoxLifecycleFuture<'a> {
                Box::pin(async { Ok(()) })
            }

            fn shutdown<'a>(&'a mut self, _: &'a LifecycleContext) -> BoxLifecycleFuture<'a> {
                Box::pin(async { Ok(()) })
            }
        }

        accepts_lifecycle(&mut Component);
    }
}
