use bevy::prelude::*;
use bevy::render::error_handler::{RenderError, RenderErrorHandler, RenderErrorPolicy};

type OwnerHandler = for<'a> fn(&'a RenderError, &'a mut World, &'a mut World) -> RenderErrorPolicy;

#[derive(Resource)]
struct PreviousHandler(OwnerHandler);

#[derive(Resource)]
pub(crate) struct RendererFault(pub(crate) String);

pub(crate) fn install(world: &mut World) {
    if world.contains_resource::<PreviousHandler>() {
        return;
    }
    let Some(handler) = world.get_resource::<RenderErrorHandler>() else {
        return;
    };
    let previous = handler.0;
    world.insert_resource(PreviousHandler(previous));
    world.insert_resource(RenderErrorHandler(handle));
}

fn handle(error: &RenderError, main: &mut World, render: &mut World) -> RenderErrorPolicy {
    if !main.contains_resource::<RendererFault>() {
        main.insert_resource(RendererFault(format!(
            "shared renderer GPU failure: {:?}: {}",
            error.ty, error.description
        )));
    }
    let previous = main.resource::<PreviousHandler>().0;
    previous(error, main, render)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::error_handler::ErrorType;

    #[derive(Resource, Default)]
    struct Calls(usize);

    fn owner(_: &RenderError, main: &mut World, render: &mut World) -> RenderErrorPolicy {
        main.resource_mut::<Calls>().0 += 1;
        render.resource_mut::<Calls>().0 += 1;
        RenderErrorPolicy::StopRendering
    }

    #[test]
    fn renderer_failure_preserves_owner_policy_and_first_fault() {
        let mut main = World::new();
        let mut render = World::new();
        main.init_resource::<Calls>();
        render.init_resource::<Calls>();
        main.insert_resource(RenderErrorHandler(owner));
        install(&mut main);
        install(&mut main);
        for description in ["original loss", "later error"] {
            let error = RenderError {
                ty: ErrorType::DeviceLost,
                description: description.into(),
                source: None,
            };
            let handler = main.resource::<RenderErrorHandler>().0;
            assert!(matches!(
                handler(&error, &mut main, &mut render),
                RenderErrorPolicy::StopRendering
            ));
        }
        assert_eq!(main.resource::<Calls>().0, 2);
        assert_eq!(render.resource::<Calls>().0, 2);
        assert!(main.resource::<RendererFault>().0.contains("original loss"));
        assert!(!main.resource::<RendererFault>().0.contains("later error"));
    }

    #[test]
    fn headless_world_does_not_install_a_renderer_policy() {
        let mut world = World::new();
        install(&mut world);
        assert!(!world.contains_resource::<RenderErrorHandler>());
        assert!(!world.contains_resource::<PreviousHandler>());
    }
}
