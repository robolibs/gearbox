use bevy::prelude::*;
use bevy::render::{
    GpuResourceAppExt, Render, RenderApp, RenderSystems,
    renderer::{RenderDevice, RenderQueue},
};

#[derive(Resource, Default)]
struct Frames {
    pending: [Option<wgpu::SubmissionIndex>; 2],
    next: usize,
}

fn replace<T>(pending: &mut [Option<T>; 2], next: &mut usize, value: T) -> Option<T> {
    let previous = pending[*next].replace(value);
    *next = (*next + 1) % pending.len();
    previous
}

pub(super) fn install(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .init_gpu_resource::<Frames>()
        .add_systems(Render, complete.in_set(RenderSystems::Cleanup));
}

fn complete(mut frames: ResMut<Frames>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let frames = &mut *frames;
    let submitted = queue.submit([]);
    if let Some(oldest) = replace(&mut frames.pending, &mut frames.next, submitted) {
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(oldest),
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .expect("headless GPU submission failed to complete within 10 seconds");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_gpu_waits_oldest_submission_without_dropping_frames() {
        let mut pending = [None, None];
        let mut next = 0;
        for frame in 0_u64..1000 {
            assert_eq!(
                replace(&mut pending, &mut next, frame),
                frame.checked_sub(2)
            );
        }
        assert_eq!(pending, [Some(998), Some(999)]);
    }

    #[test]
    fn headless_gpu_reinitialization_resets_submission_tracking() {
        let mut app = App::new();
        app.insert_sub_app(RenderApp, bevy::app::SubApp::new());
        install(&mut app);
        let render = app.sub_app_mut(RenderApp);
        render.world_mut().insert_resource(Frames {
            next: 1,
            ..Default::default()
        });
        render.world_mut().run_schedule(bevy::render::RenderStartup);
        let frames = render.world().resource::<Frames>();
        assert_eq!(frames.next, 0);
        assert!(frames.pending.iter().all(Option::is_none));
    }
}
