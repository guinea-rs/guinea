#![cfg(feature = "allocations")]

use guinea_trace::allocations::{
    Allocator, COMPLETED_CAUSE_LIMIT, Config, Profiler, allocator, heap, of_cause,
};
use guinea_trace::{Point, enter, resume};

#[global_allocator]
static ALLOCATOR: Allocator = allocator(std::alloc::System);

fn churn(count: usize) {
    for _ in 0..count {
        let event = enter(|| Point::Action { message: "Churn" });
        let bytes = Box::new([0_u8; 64]);
        std::hint::black_box(&bytes);
        drop(bytes);
        drop(event);
    }
}

#[test]
fn event_statistics_plateau_without_a_trace_sink_and_keep_live_origins() {
    let profiler = Profiler::install(Config {
        history_capacity: 4096,
    })
    .unwrap();
    let owner = enter(|| Point::Action {
        message: "Long-lived allocation",
    });
    let owner_id = owner.id();
    let retained = Box::new([0_u8; 512]);
    std::hint::black_box(&retained);
    drop(owner);

    churn(20_000);
    let warm = heap().live_bytes;
    for _ in 0..8 {
        churn(20_000);
        let stats = profiler.causes();
        assert!(
            stats.len() <= COMPLETED_CAUSE_LIMIT + 1,
            "retained {} causes",
            stats.len()
        );
        drop(stats);
        assert!(
            heap().live_bytes <= warm + 256 * 1024,
            "event metadata kept growing"
        );
        assert_eq!(of_cause(owner_id).unwrap().live_bytes, 512);
    }

    // A free on another thread still updates the original cause, even after
    // far more events than the completed-statistics retention limit.
    std::thread::spawn(move || drop(retained)).join().unwrap();
    let stats = of_cause(owner_id).unwrap();
    assert_eq!(stats.allocated_bytes, 512);
    assert_eq!(stats.freed_bytes, 512);
    assert_eq!(stats.live_bytes, 0);

    // Reusing a recently completed cause continues its existing totals.
    let current = resume(Some(owner_id));
    let again = Box::new([0_u8; 128]);
    std::hint::black_box(&again);
    drop(again);
    assert_eq!(of_cause(owner_id).unwrap().allocated_bytes, 640);
    drop(current);
    churn(20_000);
    assert!(of_cause(owner_id).is_none());
    assert!(profiler.causes().len() <= COMPLETED_CAUSE_LIMIT);
    assert!(profiler.take_history().events.is_empty());

    #[cfg(feature = "perfetto")]
    perfetto_still_exports_completed_event_totals();
}

#[cfg(feature = "perfetto")]
fn perfetto_still_exports_completed_event_totals() {
    use guinea_trace::perfetto::Recording;
    use guinea_trace::{Sink, record_into};
    use perfetto_protos::trace::Trace;
    use perfetto_protos::track_event::track_event::Type;
    use protobuf::Message;

    let sink = Sink::new();
    let recording = Recording::start(&sink, Vec::new()).unwrap();
    let current = record_into(&sink);
    let event = enter(|| Point::Action {
        message: "Recorded allocation",
    });
    let block = Box::new([0_u8; 256]);
    std::hint::black_box(&block);
    drop(block);
    drop(event);
    drop(current);
    let bytes = recording.finish().unwrap();
    let trace = Trace::parse_from_bytes(&bytes).unwrap();
    let end = trace
        .packet
        .iter()
        .filter(|packet| packet.has_track_event())
        .map(|packet| packet.track_event())
        .find(|event| event.type_() == Type::TYPE_SLICE_END)
        .unwrap();
    for (name, expected) in [
        ("memory_allocated_bytes", 256),
        ("memory_freed_bytes", 256),
        ("memory_live_bytes_at_end", 0),
    ] {
        let annotation = end
            .debug_annotations
            .iter()
            .find(|arg| arg.name() == name)
            .unwrap();
        assert_eq!(annotation.uint_value(), expected);
    }
}
