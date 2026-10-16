use super::*;
use std::time::Duration;

fn completed(slot: usize, pixel: Texel, count: usize) -> CompletedImage {
    CompletedImage { slot, pixels: vec![pixel; count], drawing_result: Ok(()) }
}

#[test]
fn preparation_clears_retained_pixels_and_reuses_capacity_across_resizes() {
    let mut pixels = vec![Texel { rgb: [0.5; 3], opacity: 0.8 }; 64];
    let address = pixels.as_ptr();
    for count in [64, 16, 0, 32, 64] {
        prepare_image(&mut pixels, count);
        assert_eq!(pixels.len(), count);
        assert!(pixels.iter().all(|pixel| *pixel == Texel::default()));
        assert_eq!(pixels.as_ptr(), address);
        pixels.fill(Texel { rgb: [0.5; 3], opacity: 0.8 });
    }
    prepare_image(&mut pixels, 129);
    assert_eq!(pixels.len(), 129);
    assert!(pixels.iter().all(|pixel| *pixel == Texel::default()));
}

#[test]
fn first_completion_becomes_the_accumulator_without_copying_any_allocation() {
    let red = Texel { rgb: [0.4, 0.0, 0.0], opacity: 0.4 };
    let blue = Texel { rgb: [0.0, 0.0, 0.5], opacity: 0.5 };
    let green = Texel { rgb: [0.0, 0.3, 0.0], opacity: 0.3 };
    let mut expected = [red];
    blend_screen_images(&mut expected, &[blue]);
    blend_screen_images(&mut expected, &[green]);

    for order in [[2, 0, 1], [1, 2, 0], [0, 1, 2]] {
        let (sender, receiver) = mpsc::sync_channel(3);
        let mut addresses = [std::ptr::null(); 3];
        for slot in order {
            let result = completed(slot, [red, blue, green][slot], 1);
            addresses[slot] = result.pixels.as_ptr();
            sender.send(result).unwrap();
        }
        drop(sender);
        let mut buffers = vec![Vec::new(); 3];
        let result_slot = collect_images(receiver, &mut buffers, &mut StepTimes::default()).unwrap();
        assert_eq!(result_slot, order[0]);
        for (buffer, address) in buffers.iter().zip(addresses) { assert_eq!(buffer.as_ptr(), address); }
        for (actual, expected) in buffers[result_slot][0].rgb.into_iter().chain([buffers[result_slot][0].opacity])
            .zip(expected[0].rgb.into_iter().chain([expected[0].opacity]))
        {
            assert!((actual - expected).abs() < 1e-6);
        }
    }
}

#[test]
fn receiver_merges_before_all_workers_have_finished() {
    // An invalid second image must fail immediately, even while a third sender keeps the channel open.
    // A collect-all-then-merge regression would time out here. Dropping sender still lets the test clean up.
    let (sender, receiver) = mpsc::sync_channel(3);
    let (finished, result) = mpsc::channel();
    let collector = std::thread::spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            collect_images(receiver, &mut vec![Vec::new(); 3], &mut StepTimes::default()).unwrap();
        }));
        finished.send(outcome.is_err()).unwrap();
    });
    sender.send(completed(0, Texel::default(), 1)).unwrap();
    sender.send(completed(1, Texel::default(), 2)).unwrap();
    let outcome = result.recv_timeout(Duration::from_secs(5));
    drop(sender);
    collector.join().unwrap();
    assert!(outcome.unwrap());
}

#[test]
fn failure_drains_remaining_images_and_preserves_the_original_panic() {
    let (sender, receiver) = mpsc::sync_channel(3);
    sender.send(completed(2, Texel::default(), 7)).unwrap();
    let mut failed = completed(0, Texel::default(), 7);
    failed.drawing_result = Err(Box::new("original worker failure"));
    sender.send(failed).unwrap();
    sender.send(completed(1, Texel::default(), 7)).unwrap();
    drop(sender);
    let mut buffers = vec![Vec::new(); 3];
    let failure = collect_images(receiver, &mut buffers, &mut StepTimes::default()).unwrap_err();
    assert_eq!(failure.downcast_ref::<&str>(), Some(&"original worker failure"));
    assert!(buffers.iter().all(|buffer| buffer.len() == 7));
}
