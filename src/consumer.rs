//! Data consumers
//!
//! The goal of data producers is to parse data depending on the previous result.
//! It can be used to selectively seek in a file.
//!
//! ## Example
//!
//! This consumer will take 4 samples from the input, print them, then stop
//!
//! ```rust
//!  use nom::{IResult,Needed,MemProducer,Consumer,ConsumerState};
//!  use std::str;
//!
//!  struct TestPrintConsumer {
//!    counter: usize
//!  }
//!
//!  impl TestPrintConsumer {
//!    fn new() -> TestPrintConsumer {
//!      TestPrintConsumer { counter: 0 }
//!    }
//!  }
//!
//!  fn take4(i:&[u8]) -> IResult<&[u8], &[u8]>{
//!    if i.len() < 4 {
//!      IResult::Incomplete(Needed::Size(4))
//!    } else {
//!      IResult::Done(&i[4..],&i[0..4])
//!    }
//!  }
//!
//!  // Return ConsumerState::Await if it needs more data, or ConsumerDone when it ends
//!  impl Consumer for TestPrintConsumer {
//!    fn consume(&mut self, input: &[u8]) -> ConsumerState {
//!      match take4(input) {
//!        IResult::Error(a)      => ConsumerState::ConsumerError(0),
//!        IResult::Incomplete(a) => ConsumerState::Await(0, 4),
//!        IResult::Done(i, o)    => {
//!          println!("{} -> {}", self.counter, str::from_utf8(o).unwrap());
//!          self.counter = self.counter + 1;
//!          if self.counter <= 4 {
//!            ConsumerState::Await(4, 4)
//!          } else {
//!            ConsumerState::ConsumerDone
//!          }
//!        }
//!      }
//!    }
//!
//!    fn failed(&mut self, error_code: u32) {
//!      println!("failed with error code {}", error_code);
//!    }
//!
//!    fn end(&mut self) {
//!      println!("finished");
//!    }
//!  }
//!
//!  // It can consume data directly from a producer
//!  let mut p = MemProducer::new(b"abcdefghijklmnopqrstuvwx", 4);
//!  let mut c = TestPrintConsumer::new();
//!  c.run(&mut p);
//! ```

use self::ConsumerState::*;
use producer::Producer;
use producer::ProducerState::*;
use std::io::SeekFrom;

/// Holds the current state of the consumer
///
/// * Await(consumed, needed input data size) if more data is needed
///
/// * Seek(consumed, new position, needed input data size) if the consumer must move back or forth
///
/// * Incomplete if there is not enough data but not enough information for Await or Seek
///
/// * ConsumerDone if the consumer does not need anymore data to be parsed
///
/// * ConsumerError(error code) when something went wrong
#[derive(Debug,PartialEq,Eq,Clone,Copy)]
pub enum ConsumerState<E=u32> {
  Await(
    usize,    // consumed
    usize     // needed buffer size
  ),
  Seek(
    usize,    // consumed
    SeekFrom, // new position
    usize     // needed buffer size
  ),
  Incomplete,
  ConsumerDone,
  ConsumerError(E)
}

/// Implement the consume method, taking a byte array as input and returning a consumer state
///
/// The run function takes care of continuing or not
pub trait Consumer<E=u32> {
  fn consume(&mut self, input: &[u8]) -> ConsumerState<E>;
  fn failed(&mut self, error_code: E);
  fn end(&mut self);

  fn run(&mut self, producer: &mut Producer) {
    let mut acc: Vec<u8>       = Vec::new();
    let mut position           = 0;
    let mut should_seek        = false;
    let mut consumed:usize     = 0;
    let mut needed:usize       = 0;
    let mut seek_from:SeekFrom = SeekFrom::Current(0);
    let mut eof = false;
    let mut end = false;
    let mut err: Option<E> = None;

    loop {
      //self.getDataFromProducer(producer, seek_from, needed, acc);
      if !should_seek && acc.len() - consumed >= needed {
        //println!("buffer is large enough, skipping");
        let mut tmp = Vec::new();
        //println!("before:\n{}", acc.to_hex(16));
        //println!("after:\n{}", (&acc[consumed..acc.len()]).to_hex(16));
        tmp.extend(acc[consumed..acc.len()].iter().cloned());
        acc.clear();
        acc = tmp;
      } else {
        if should_seek {
          let _ = producer.seek(seek_from);
          //println!("seeking: {:?}", pos);
          should_seek = false;
          acc.clear();
        } else {
          let mut tmp = Vec::new();
          tmp.extend(acc[consumed..acc.len()].iter().cloned());
          acc.clear();
          acc = tmp;
        }

        loop {
          let state   = producer.produce();
          match state {
            Data(v) => {
              //println!("got data: {} bytes", v.len());
              acc.extend(v.iter().cloned());
              position = position + v.len();
            },
            Eof(v) => {
              if v.is_empty() {
                //println!("eof empty");
                //eof = true;
                self.end();
                return
              } else {
                //println!("eof with {} bytes", v.len());
                eof = true;
                acc.extend(v.iter().cloned());
                position = position + v.len();
                break;
              }
            }
            ProducerError(_) => {break;}
            Continue => { continue; }
          }
          //println!("acc size: {}", acc.len());
          if acc.len() >= needed { break; }
        }
      }

      //println!("full:\n{}", acc.to_hex(8));
      //println!("truncated:\n{}", (&acc[0..needed]).to_hex(16));
      match self.consume(&acc[0..needed]) {
        ConsumerError(e) => {
          //println!("consumer error, stopping: {}", e);
          err = Some(e);
        },
        ConsumerDone => {
          //println!("data, done");
          end = true;
        },
        Seek(consumed_bytes, sf, needed_bytes) => {
          //println!("Seek: consumed {} bytes, got {:?} and asked {} bytes", consumed_bytes, sf, needed_bytes);
          seek_from = match sf {
            SeekFrom::Current(i) => SeekFrom::Current(i - (acc.len() - needed) as i64),
            a => a
          };
          should_seek = true;
          consumed = consumed_bytes;
          needed   = needed_bytes;
        },
        Await(consumed_bytes, needed_bytes) => {
          //println!("consumed: {} bytes | needed: {} bytes", consumed_bytes, needed_bytes);
          consumed = consumed_bytes;
          needed   = needed_bytes;
        },
        Incomplete => {
          //println!("incomplete");
        }
      }
      if let Some(e) = err {
        self.failed(e);
        break;
      }
      if eof {
        self.end();
        break;
      }
      if end {
        self.end();
        break;
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use super::ConsumerState::*;
  use producer::MemProducer;
  use internal::{Needed,IResult};
  use std::str;

#[macro_export]
macro_rules! take(
  ($name:ident $count:expr) => (
    fn $name(i:&[u8]) -> IResult<&[u8], &[u8]>{
      if i.len() < $count {
        IResult::Incomplete(Needed::Size($count))
      } else {
        IResult::Done(&i[$count..],&i[0..$count])
      }
    }
  )
);

  struct TestPrintConsumer {
    counter: usize,
    ended:   bool
  }

  impl TestPrintConsumer {
    fn new() -> TestPrintConsumer {
      TestPrintConsumer { counter: 0, ended: false }
    }
  }

  take!(take4 4);

  impl Consumer for TestPrintConsumer {
    fn consume(&mut self, input: &[u8]) -> ConsumerState {
      match take4(input) {
        IResult::Error(_)      => ConsumerError(0),
        IResult::Incomplete(_) => Await(0, 4),
        IResult::Done(_, o)    => {
          println!("{} -> {}", self.counter, str::from_utf8(o).unwrap());
          self.counter = self.counter + 1;
          if self.counter <= 4 {
            Await(4, 4)
          } else {
            ConsumerDone
          }
        }
      }
    }

    fn end(&mut self) {
      assert_eq!(self.counter, 5);
      self.ended = true;
    }

    fn failed(&mut self, error_code: u32) {
      println!("failed with error code: {}", error_code);
    }
  }

  #[test]
  fn pull() {
    let mut p = MemProducer::new(&b"abcdefghijklmnopqrstuvwx"[..], 4);
    let mut c = TestPrintConsumer::new();
    c.run(&mut p);

    assert!(c.ended);
  }

  struct StringErrorConsumer { failed_called : bool }

  impl Consumer<&'static str> for StringErrorConsumer {
      fn end(&mut self) {
      }

      fn failed(&mut self, error: &str) {
        self.failed_called = true;
        assert_eq!(error, "error");
      }
    fn consume(&mut self, _: &[u8]) -> ConsumerState<&'static str> {
        ConsumerState::ConsumerError("error")
    }
  }

  #[test]
  fn string_error() {
      let mut p = MemProducer::new(b"a", 1);
      let mut c = StringErrorConsumer{ failed_called: false };
      c.run(&mut p);
      assert_eq!(c.failed_called, true);
  }
}
