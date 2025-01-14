use async_lock::RefCell;
use async_std::task::sleep;
use futures_lite::future;
use std::time::Duration;

#[test]
fn test_refcell() {
    let vec = RefCell::new(Vec::new());

    // insert entry in vec
    let insert_fut = async {
        let mut idx = 1;
        loop {
            {
                let mut vec = vec.borrow_mut().await;
                let new_val = (((idx + 10) * 4) + idx + 3) / 3;
                sleep(Duration::from_micros(500)).await;
                println!("Insert new val :{new_val}");
                vec.push(new_val);
            }
            sleep(Duration::from_secs(2)).await;
            idx += 1;
            if idx == 10 {
                sleep(Duration::from_secs(2)).await;
                break;
            }
        }

        sleep(Duration::from_secs(2)).await;
        vec.borrow_mut().await.clear();
    };

    // print the content of vec
    let print_fut = async {
        sleep(Duration::from_micros(100)).await;
        loop {
            {
                let vec = vec.borrow().await;
                if vec.is_empty() {
                    return;
                }
                println!("Vec :");
                vec.iter()
                    .enumerate()
                    .for_each(|(idx, val)| println!("\t{idx}: {val}"));
            }
            sleep(Duration::from_micros(1500)).await;
        }
    };
    future::block_on(future::zip(insert_fut, print_fut));
}
