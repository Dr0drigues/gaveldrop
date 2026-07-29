//! Reserving a port for a living subject.

use std::net::TcpListener;

/// A port nobody is listening on, released before returning.
///
/// Binding port 0 lets the kernel choose, which is the only way to avoid arguing with whatever else
/// is running on the machine. The listener is then dropped: the port is reserved *for the subject*,
/// and holding it open would make the subject fail to bind.
///
/// That leaves a window between the release and the subject's bind. It is the standard race for this
/// pattern, and it is recorded as an accepted gap — the alternative, handing an already-open socket
/// to a child, works only for a subject we compile ourselves, which is exactly the assumption this
/// project refuses to make.
pub fn reserve() -> Result<u16, std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reserved_port_is_usable_by_someone_else_afterwards() {
        let port = reserve().unwrap();

        std::net::TcpListener::bind(("127.0.0.1", port)).expect(
            "the port must be released, not held: we reserve it so the subject can bind it, and \
             a listener we keep open would make the subject fail to start",
        );
    }

    #[test]
    fn two_reservations_do_not_collide() {
        let first = reserve().unwrap();
        let held = std::net::TcpListener::bind(("127.0.0.1", first)).unwrap();
        let second = reserve().unwrap();
        drop(held);

        assert_ne!(
            first, second,
            "two cases running in sequence must not be handed the same port while the first is \
             still bound, or the second subject fails to start for a reason belonging to \
             neither case"
        );
    }

    #[test]
    fn a_reserved_port_is_not_a_privileged_one() {
        assert!(
            reserve().unwrap() >= 1024,
            "a port below 1024 needs root, so a case would fail on a developer's machine for a \
             reason that has nothing to do with the subject"
        );
    }
}
