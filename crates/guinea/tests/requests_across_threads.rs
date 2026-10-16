//! Requests asked on threads of their own and answered on the UI thread, with
//! a real UI loop rather than the harness's one thread: a binary of its own,
//! since a `UiLoop` makes the thread that installs it the process's UI thread.

use std::time::Duration;

use guinea_core::__private::tokio;
use guinea_core::scope::ScopeTree;
use guinea_core::ui_thread::UiLoop;

#[derive(Clone, Debug, guinea::Request)]
#[request(reply = Sum)]
struct Add(u32, u32);

#[derive(Clone, Debug)]
struct Sum(u32);

const THREADS: u32 = 8;
const EACH: u32 = 25;

#[test]
fn requests_from_many_threads_at_once_are_all_answered() {
    let ui = UiLoop::install();
    let tree = ScopeTree::new();
    let _answers = tree.bus().answer_fn(|Add(a, b): Add| Sum(a + b));
    let scope = tree.scope();

    let asking: Vec<_> = (0..THREADS)
        .map(|thread| {
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .unwrap();

                runtime.block_on(async {
                    for i in 0..EACH {
                        let Sum(sum) = scope
                            .request(Add(thread, i), Duration::from_secs(5))
                            .await
                            .unwrap_or_else(|error| panic!("request {thread}/{i}: {error}"));
                        assert_eq!(sum, thread + i);
                    }
                });
            })
        })
        .collect();

    let answered = ui.run_until(|| asking.iter().all(|asker| asker.is_finished()), Duration::from_secs(10));
    assert!(answered, "the requests were not all answered: something waits on itself");

    for asker in asking {
        asker.join().unwrap();
    }
}
