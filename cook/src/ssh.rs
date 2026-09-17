use std::error::Error;
use std::future::Future;
use std::ops::Deref;
use std::process::{ExitStatus, Output};
use std::sync::{Arc, Mutex};

use openssh::{Child, Stdio};
use openssh_mux_client_error::Error as MuxError;
use openssh_sftp_client::{Sftp as InnerSftp, SftpOptions};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// One SSH connection with adaptive session admission for commands and SFTP.
pub struct Session {
    inner: Arc<openssh::Session>,
    admission: Admission,
}

impl Session {
    pub async fn connect(host: &str) -> Result<Self, openssh::Error> {
        let inner = openssh::Session::connect_mux(host, openssh::KnownHosts::Strict).await?;
        Ok(Self {
            inner: Arc::new(inner),
            admission: Admission::new(host),
        })
    }

    pub fn command(&self, program: impl AsRef<str>) -> Command<'_> {
        Command {
            session: self,
            program: program.as_ref().into(),
            args: Vec::new(),
        }
    }

    pub async fn sftp(&self) -> Result<Sftp, openssh_sftp_client::Error> {
        let (inner, permit) = self
            .admission
            .open(|| InnerSftp::from_clonable_session(self.inner.clone(), SftpOptions::new()))
            .await?;
        Ok(Sftp { inner, _permit: permit })
    }
}

pub struct Command<'a> {
    session: &'a Session,
    program: String,
    args: Vec<String>,
}

impl<'a> Command<'a> {
    pub fn arg(&mut self, arg: impl AsRef<str>) -> &mut Self {
        self.args.push(arg.as_ref().into());
        self
    }

    pub fn args<I, A>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = A>,
        A: AsRef<str>,
    {
        self.args.extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    async fn spawn(
        &self,
        capture: bool,
    ) -> Result<(Child<&'a openssh::Session>, OwnedSemaphorePermit), openssh::Error> {
        let session: &'a openssh::Session = &self.session.inner;
        self.session
            .admission
            .open(|| async {
                let mut command = session.command(&self.program);
                command.args(&self.args);
                if capture {
                    command
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                }
                command.spawn().await
            })
            .await
    }

    pub async fn output(&mut self) -> Result<Output, openssh::Error> {
        let (child, _permit) = self.spawn(true).await?;
        child.wait_with_output().await
    }

    pub async fn status(&mut self) -> Result<ExitStatus, openssh::Error> {
        let (child, _permit) = self.spawn(false).await?;
        child.wait().await
    }
}

pub struct Sftp {
    inner: InnerSftp,
    _permit: OwnedSemaphorePermit,
}

impl Deref for Sftp {
    type Target = InnerSftp;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Sftp {
    pub async fn close(self) -> Result<(), openssh_sftp_client::Error> {
        let Self { inner, _permit } = self;
        inner.close().await
    }
}

struct Admission {
    host: String,
    slots: Arc<Semaphore>,
    capacity: Mutex<usize>,
}

impl Admission {
    fn new(host: &str) -> Self {
        Self {
            host: host.into(),
            slots: Arc::new(Semaphore::new(Semaphore::MAX_PERMITS)),
            capacity: Mutex::new(Semaphore::MAX_PERMITS),
        }
    }

    async fn open<T, E, F, Fut>(&self, mut attempt: F) -> Result<(T, OwnedSemaphorePermit), E>
    where
        E: Error + 'static,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let mut serial_refusals = 0;
        loop {
            let permit = self
                .slots
                .clone()
                .acquire_owned()
                .await
                .expect("admission is never closed");
            match attempt().await {
                Ok(value) => return Ok((value, permit)),
                Err(error) if session_refused(&error) => {
                    if !self.reduce(permit) {
                        serial_refusals += 1;
                        if serial_refusals == 3 {
                            return Err(error);
                        }
                    }
                    // The mux can report exit before sshd releases the remote session.
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn reduce(&self, permit: OwnedSemaphorePermit) -> bool {
        let mut capacity = self.capacity.lock().unwrap();
        if *capacity == Semaphore::MAX_PERMITS {
            tracing::warn!(
                host = %self.host,
                "SSH session open refused (possibly MaxSessions); reducing concurrency and queuing pending work"
            );
        }
        let was_serial = *capacity == 1;
        // Remove unused capacity first, then retire the rejected opening's slot.
        // Outstanding opens keep their permits; each further refusal shrinks the limit.
        *capacity -= self.slots.forget_permits(self.slots.available_permits());
        if *capacity > 1 {
            *capacity -= 1;
            permit.forget();
        }
        // Keep one slot so pending work can progress or report a permanent refusal.
        !was_serial
    }
}

fn session_refused(mut error: &(dyn Error + 'static)) -> bool {
    loop {
        if let Some(openssh::Error::SshMux(MuxError::RequestFailure(message))) = error.downcast_ref() {
            return message.as_ref() == "Session open refused by peer";
        }
        match error.source() {
            Some(source) => error = source,
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn refused() -> openssh::Error {
        openssh::Error::SshMux(MuxError::RequestFailure("Session open refused by peer".into()))
    }

    #[tokio::test]
    async fn saturation_learns_capacity_and_queues_without_replaying_work() {
        for limit in [1, 3, 10] {
            let admission = Admission::new("mock");
            let server = Arc::new(Semaphore::new(limit));
            let attempts = AtomicUsize::new(0);
            let executed = AtomicUsize::new(0);
            for wave in 0..2 {
                let before = attempts.load(Ordering::SeqCst);
                tokio::time::timeout(
                    Duration::from_secs(5),
                    futures::future::join_all((0..32).map(|_| async {
                        let (remote, permit) = admission
                            .open(|| async {
                                attempts.fetch_add(1, Ordering::SeqCst);
                                tokio::task::yield_now().await;
                                server.clone().try_acquire_owned().map_err(|_| refused())
                            })
                            .await
                            .unwrap();
                        executed.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        drop(remote);
                        drop(permit);
                    })),
                )
                .await
                .expect("queued opens stalled");
                assert_eq!(*admission.capacity.lock().unwrap(), limit);
                assert_eq!(admission.slots.available_permits(), limit);
                if wave == 1 {
                    assert_eq!(attempts.load(Ordering::SeqCst) - before, 32);
                }
            }
            assert_eq!(executed.load(Ordering::SeqCst), 64);
        }
    }

    #[tokio::test]
    async fn permanent_refusal_stops_at_serial_execution() {
        let admission = Admission::new("mock");
        let attempts = AtomicUsize::new(0);
        let result = admission
            .open(|| async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(refused())
            })
            .await;
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 4);
        assert_eq!(admission.slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn serial_refusal_recovers_after_remote_cleanup() {
        let admission = Admission::new("mock");
        let attempts = AtomicUsize::new(0);
        let (_, permit) = admission
            .open(|| async {
                if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(refused())
                } else {
                    Ok(())
                }
            })
            .await
            .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        drop(permit);
        assert_eq!(admission.slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn unrelated_errors_are_not_retried_or_throttled() {
        for error in [
            openssh::Error::Disconnected,
            openssh::Error::SshMux(MuxError::PermissionDenied("Permission denied".into())),
            openssh::Error::SshMux(MuxError::RequestFailure("other failure".into())),
        ] {
            let admission = Admission::new("mock");
            let mut error = Some(error);
            assert!(
                admission
                    .open(|| std::future::ready(Err::<(), _>(error.take().expect("unexpected retry"))))
                    .await
                    .is_err()
            );
            assert_eq!(*admission.capacity.lock().unwrap(), Semaphore::MAX_PERMITS);
        }
        let wrapped = openssh_sftp_client::Error::from(refused());
        assert!(session_refused(&wrapped));
    }

    #[tokio::test]
    async fn cancellation_releases_active_and_queued_permits() {
        let admission = Admission::new("mock");
        let first = admission.slots.clone().acquire_owned().await.unwrap();
        let rejected = admission.slots.clone().acquire_owned().await.unwrap();
        assert!(admission.reduce(rejected));
        assert_eq!(*admission.capacity.lock().unwrap(), 1);
        let queued = admission.open(|| async { Ok::<_, openssh::Error>(()) });
        assert!(tokio::time::timeout(Duration::from_millis(10), queued).await.is_err());
        drop(first);
        let opening = admission.open(std::future::pending::<Result<(), openssh::Error>>);
        assert!(tokio::time::timeout(Duration::from_millis(10), opening).await.is_err());
        let (_, permit) = tokio::time::timeout(
            Duration::from_secs(1),
            admission.open(|| async { Ok::<_, openssh::Error>(()) }),
        )
        .await
        .unwrap()
        .unwrap();
        drop(permit);
        assert_eq!(admission.slots.available_permits(), 1);
    }

    #[tokio::test]
    #[ignore = "requires COOK_TEST_SSH_HOST; runs only sleep commands and opens SFTP"]
    async fn live_ssh_session_pressure() {
        let host = std::env::var("COOK_TEST_SSH_HOST").expect("set COOK_TEST_SSH_HOST");
        let session = Session::connect(&host).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(30),
            futures::future::join_all((0..32).map(|i| {
                let session = &session;
                async move {
                    if i % 2 == 0 {
                        let output = session.command("sleep").arg("0.1").output().await.unwrap();
                        assert!(output.status.success());
                    } else {
                        let sftp = session.sftp().await.unwrap();
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        sftp.close().await.unwrap();
                    }
                }
            })),
        )
        .await
        .expect("SSH queue stalled");
        assert!(*session.admission.capacity.lock().unwrap() < Semaphore::MAX_PERMITS);
        assert!(session.command("true").status().await.unwrap().success());
    }
}
