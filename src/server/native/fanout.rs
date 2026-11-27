//! Run several futures at the same time in the current task.
//!
//! The segmented path keeps a small fixed number of requests in flight. Each
//! one borrows a handle owned by the caller, so they cannot be spawned and
//! the whole set has to be polled here.

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

/// Poll every future until all have finished. The outputs come back in the
/// order the futures were given, not in the order they finished.
pub async fn all<F: Future>(futures: Vec<F>) -> Vec<F::Output> {
    let mut futures: Vec<Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut slots: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();

    let outputs = std::future::poll_fn(move |context| {
        let mut pending = false;
        for (slot, future) in slots.iter_mut().zip(futures.iter_mut()) {
            if slot.is_some() {
                continue;
            }
            match future.as_mut().poll(context) {
                Poll::Ready(value) => *slot = Some(value),
                Poll::Pending => pending = true,
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(std::mem::take(&mut slots))
        }
    })
    .await;

    outputs
        .into_iter()
        .map(|slot| slot.expect("every future was polled to a value"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn delayed(value: u8, millis: u64) -> u8 {
        tokio::time::sleep(Duration::from_millis(millis)).await;
        value
    }

    #[tokio::test]
    async fn every_future_runs_and_the_order_is_kept() {
        let started = std::time::Instant::now();
        let values = all(vec![delayed(1, 60), delayed(2, 10), delayed(3, 30)]).await;
        assert_eq!(values, vec![1, 2, 3], "the answer order follows the input");
        assert!(
            started.elapsed() < Duration::from_millis(90),
            "the futures ran at the same time, not one after the other"
        );
    }

    #[tokio::test]
    async fn an_empty_set_finishes_at_once() {
        let values: Vec<u8> = all(Vec::<std::future::Ready<u8>>::new()).await;
        assert!(values.is_empty());
    }

    #[tokio::test]
    async fn one_failure_does_not_stop_the_others() {
        type Answer = std::pin::Pin<Box<dyn Future<Output = Result<u8, u8>>>>;
        let answers: Vec<Answer> = vec![
            Box::pin(async { Ok::<u8, u8>(1) }),
            Box::pin(async { Err::<u8, u8>(2) }),
            Box::pin(async { Ok::<u8, u8>(3) }),
        ];
        let values: Vec<Result<u8, u8>> = all(answers).await;
        assert_eq!(values, vec![Ok(1), Err(2), Ok(3)]);
    }
}
