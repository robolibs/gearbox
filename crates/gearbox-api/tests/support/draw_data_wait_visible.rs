use super::*;
use gearbox_api::{Client, DrawDataError, HostBus, HostConfig, IdentitySource, WaitVisibleError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[path = "draw_data_wait_pipeline.rs"]
mod pipeline;

struct Harness {
    client: Client,
    requests: Arc<Mutex<Vec<DataRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Harness {
    fn new(backend: DrawingBackend) -> Self {
        let host = HostBus::new(HostConfig::new("visibility-test", "test").ephemeral()).unwrap();
        let client = Client::connect_id(
            host.endpoint_id(),
            IdentitySource::Random,
            "visibility-client",
        )
        .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker = {
            let stop = stop.clone();
            let requests = requests.clone();
            std::thread::spawn(move || {
                let lane = host.draw_data_lane();
                let mut store = StreamingStore::with_backend(lane.epoch(), backend).unwrap();
                let start = Instant::now();
                while !stop.load(Ordering::Acquire) {
                    let caps = store.capabilities();
                    let _ = lane.serve(&caps, |request, _| {
                        requests.lock().unwrap().push(request.clone());
                        store.handle(&request, start.elapsed().as_millis() as u64)
                    });
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        };
        Self {
            client,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn timeout_is_read_only_and_public_client_recovers_for_exact_success() {
    let backend = backend();
    let (key, revision) = publish(&backend, None, 2);
    let registry_revision = backend.registry().lock().unwrap().revision();
    let host = Harness::new(backend.clone());
    let mut data = host.client.draw_data().unwrap();
    assert!(matches!(
        data.wait_visible(&key, revision, None, Duration::ZERO),
        Err(WaitVisibleError::Request(DrawDataError::Timeout))
    ));
    assert!(host.requests.lock().unwrap().is_empty());
    assert!(matches!(
        data.wait_visible(&key, revision, None, Duration::from_millis(120)),
        Err(WaitVisibleError::Request(DrawDataError::Timeout))
    ));
    {
        let requests = host.requests.lock().unwrap();
        assert!(requests.len() >= 2);
        for (index, request) in requests.iter().enumerate() {
            assert_eq!(request.operation, data_op::STATUS);
            assert_eq!(request.incarnation, revision.incarnation);
            assert_eq!(request.base_generation, revision.generation);
            let query: DataStatusQuery = serde_json::from_slice(&request.metadata).unwrap();
            assert_eq!(query.generation, Some(revision.generation));
            assert_eq!(query.view, if index == 0 { None } else { Some(A) });
        }
    }
    assert_eq!(
        backend.registry().lock().unwrap().revision(),
        registry_revision
    );
    let completion = receipt(revision, A, 7, 2);
    assert!(backend.complete_render_frame(&key, completion).unwrap());
    assert_eq!(
        data.wait_visible(&key, revision, Some(A), Duration::from_secs(5))
            .unwrap(),
        completion
    );
    backend.sync_render_views(&[B], Some(B)).unwrap();
    assert!(matches!(
        data.wait_visible(&key, revision, Some(A), Duration::from_secs(5)),
        Err(WaitVisibleError::Terminal(
            GenerationVisibility::ViewGone { .. }
        ))
    ));
    backend.sync_render_views(&[], None).unwrap();
    assert!(matches!(
        data.wait_visible(&key, revision, None, Duration::from_secs(5)),
        Err(WaitVisibleError::Terminal(
            GenerationVisibility::Unavailable { .. }
        ))
    ));
    assert_eq!(
        backend.registry().lock().unwrap().revision(),
        registry_revision
    );
}
