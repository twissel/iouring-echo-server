use iou;
use std::io;
use std::os::unix::io::RawFd;
use std::slice;

pub enum OperationState<P, R> {
    Pending(P),
    Ready(R),
    Temp,
}

static RESPONSE: &str = "HTTP/1.1 200 OK
Content-Type: text/html
Connection: keep-alive
Content-Length: 6

hello
";

impl<P, R> OperationState<P, R> {
    pub fn as_pending(&self) -> &P {
        match self {
            OperationState::Pending(p) => p,
            OperationState::Ready(_) => panic!("state is ready"),
            OperationState::Temp => panic!("state is temp"),
        }
    }

    pub fn into_pending(self) -> P {
        match self {
            OperationState::Pending(p) => p,
            OperationState::Ready(_) => panic!("state is ready"),
            OperationState::Temp => panic!("state is temp"),
        }
    }

    pub fn as_pending_mut(&mut self) -> &mut P {
        match self {
            OperationState::Pending(p) => p,
            OperationState::Ready(_) => panic!("state is ready"),
            OperationState::Temp => panic!("state is temp"),
        }
    }

    pub fn as_ready(&self) -> &R {
        match self {
            OperationState::Pending(_) => panic!("state is pending"),
            OperationState::Ready(r) => r,
            OperationState::Temp => panic!("state is temp"),
        }
    }

    pub fn into_ready(self) -> R {
        match self {
            OperationState::Pending(_) => panic!("state is pending"),
            OperationState::Ready(r) => r,
            OperationState::Temp => panic!("state is temp"),
        }
    }

    pub fn take(&mut self) -> OperationState<P, R> {
        std::mem::replace(self, OperationState::Temp)
    }
}

pub enum Operation {
    Accept(OperationState<AcceptPending, AcceptReady>),
    ConnectionRead(OperationState<ConnectionReadPending, ConnectionReadReady>),
    ConnectionWrite(OperationState<ConnectionWritePending, ConnectionWriteReady>),
}

impl Operation {
    pub fn new_accept(fd: RawFd) -> Box<Self> {
        let state = OperationState::Pending(AcceptPending { listen_fd: fd });
        Box::new(Operation::Accept(state))
    }

    pub fn new_connection_read(fd: RawFd) -> Box<Self> {
        let state = OperationState::Pending(ConnectionReadPending {
            data: ConnectionData::new(fd),
        });
        Box::new(Operation::ConnectionRead(state))
    }

    pub unsafe fn from_cqe(
        cqe: iou::CompletionQueueEvent<'_>,
    ) -> Result<Box<Operation>, (Option<Box<Operation>>, io::Error)> {
        let data: *mut Operation = cqe.user_data() as _;
        if !data.is_null() {
            let mut op = Box::from_raw(data);
            match *op {
                Operation::Accept(ref mut state) => {
                    let accept_fd = cqe.result();
                    match accept_fd {
                        Ok(accept_fd) => {
                            *state = OperationState::Ready(AcceptReady {
                                accept_fd: accept_fd as _,
                                listen_fd: state.as_pending().listen_fd,
                            });
                            Ok(op)
                        }
                        Err(e) => Err((Some(op), e)),
                    }
                }
                Operation::ConnectionRead(ref mut state) => {
                    let bytes_read = cqe.result();
                    match bytes_read {
                        Ok(bytes_read) => {
                            let connection = state.take().into_pending();
                            *state = OperationState::Ready(ConnectionReadReady {
                                data: connection.data,
                                bytes_read,
                            });
                            Ok(op)
                        }
                        Err(e) => Err((Some(op), e)),
                    }
                }
                Operation::ConnectionWrite(ref mut state) => {
                    let bytes_wrote = cqe.result();
                    match bytes_wrote {
                        Ok(bytes_wrote) => {
                            let connection = state.take().into_pending();
                            *state = OperationState::Ready(ConnectionWriteReady {
                                data: connection.data,
                                bytes_wrote,
                            });
                            Ok(op)
                        }
                        Err(e) => Err((Some(op), e)),
                    }
                }
            }
        } else {
            Err((
                None,
                io::Error::new(io::ErrorKind::Other, "null ptr in cqe"),
            ))
        }
    }
}

pub struct AcceptPending {
    pub listen_fd: RawFd,
}

pub struct AcceptReady {
    pub listen_fd: RawFd,
    pub accept_fd: RawFd,
}

pub struct ConnectionData {
    fd: RawFd,
    read_buf: io::IoSliceMut<'static>,
    write_buf: io::IoSlice<'static>,
}

pub struct ConnectionReadPending {
    pub data: ConnectionData,
}

pub struct ConnectionReadReady {
    pub bytes_read: usize,
    pub data: ConnectionData,
}

pub struct ConnectionWritePending {
    pub data: ConnectionData,
}

pub struct ConnectionWriteReady {
    pub data: ConnectionData,
    pub bytes_wrote: usize,
}

impl ConnectionData {
    pub fn new(fd: RawFd) -> Self {
        let mut read_buf = vec![0; 4096].into_boxed_slice();
        let write_buf = RESPONSE.as_bytes().to_vec().into_boxed_slice();
        let (read_buf, write_buf) = unsafe {
            let p = (
                io::IoSliceMut::new(slice::from_raw_parts_mut(
                    read_buf.as_mut_ptr(),
                    read_buf.len(),
                )),
                io::IoSlice::new(slice::from_raw_parts(write_buf.as_ptr(), write_buf.len())),
            );
            std::mem::forget(read_buf);
            std::mem::forget(write_buf);
            p
        };

        ConnectionData {
            fd,
            read_buf,
            write_buf,
        }
    }
}

impl Drop for ConnectionData {
    fn drop(&mut self) {
        unsafe {
            nix::libc::close(self.fd);
            Box::<[u8]>::from_raw(slice::from_raw_parts_mut(
                self.write_buf.as_ptr() as *mut _,
                self.write_buf.len(),
            ));
            Box::<[u8]>::from_raw(slice::from_raw_parts_mut(
                self.read_buf.as_mut_ptr(),
                self.read_buf.len(),
            ));
        }
    }
}

pub trait PrepareOperation {
    fn prepare_operation(&mut self, op: Box<Operation>);
}

impl PrepareOperation for iou::SubmissionQueueEvent<'_> {
    fn prepare_operation(&mut self, mut op: Box<Operation>) {
        match *op {
            Operation::Accept(ref state) => {
                let pending_accept = state.as_pending();
                unsafe { self.prep_accept(pending_accept.listen_fd, None, iou::SockFlag::empty()) };
            }
            Operation::ConnectionRead(ref mut state) => {
                let pending_read = state.as_pending_mut();
                unsafe {
                    self.prep_read_vectored(
                        pending_read.data.fd,
                        slice::from_mut(&mut pending_read.data.read_buf),
                        0,
                    );
                }
            }
            Operation::ConnectionWrite(ref mut state) => {
                let pending_write = state.as_pending();
                unsafe {
                    self.prep_write_vectored(
                        pending_write.data.fd,
                        slice::from_ref(&pending_write.data.write_buf),
                        0,
                    );
                }
            }
        }
        self.set_user_data(Box::into_raw(op) as _);
    }
}
