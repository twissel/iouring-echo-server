use iou;
use std::collections::VecDeque;
use std::io;
use std::net;
use std::os::unix::io::AsRawFd;
mod operation;
use operation::*;

fn main() -> io::Result<()> {
    let mut ring = iou::IoUring::new(1024)?;
    let (mut sq, mut cq, _) = ring.queues();
    let listener = net::TcpListener::bind("127.0.0.1:9000")?;
    let mut ops = VecDeque::new();
    listener.set_nonblocking(false)?;
    let op = Operation::new_accept(listener.as_raw_fd());
    ops.push_back(op);
    loop {
        while let Some(op) = ops.pop_front() {
            let sqe = sq.next_sqe();
            match sqe {
                Some(mut sqe) => {
                    sqe.prepare_operation(op);
                }
                None => {
                    ops.push_front(op);
                    break;
                }
            }
        }

        let _ = sq.submit_and_wait(1)?;
        while let Some(cqe) = cq.peek_for_cqe() {
            let op = unsafe { Operation::from_cqe(cqe) };
            match op {
                Ok(mut op) => match *op {
                    Operation::Accept(ref state) => {
                        let accept_ready = state.as_ready();
                        let accept_fd = accept_ready.accept_fd;
                        *op = Operation::Accept(OperationState::Pending(AcceptPending {
                            listen_fd: accept_ready.listen_fd,
                        }));
                        let connection_read_op = Operation::new_connection_read(accept_fd);
                        ops.push_back(op);
                        ops.push_back(connection_read_op);
                    }
                    Operation::ConnectionRead(ref mut state) => {
                        let read_ready = state.take().into_ready();
                        if read_ready.bytes_read > 0 {
                            *op = Operation::ConnectionWrite(OperationState::Pending(
                                ConnectionWritePending {
                                    data: read_ready.data,
                                },
                            ));
                            ops.push_back(op);
                        }
                    }
                    Operation::ConnectionWrite(ref mut state) => {
                        let read_ready = state.take().into_ready();
                        *op = Operation::ConnectionRead(OperationState::Pending(
                            ConnectionReadPending {
                                data: read_ready.data,
                            },
                        ));
                        ops.push_back(op);
                    }
                },
                Err((_op, _e)) => {
                    // TODO: handle errors
                    //dbg!(e);
                }
            }
        }
    }
}
