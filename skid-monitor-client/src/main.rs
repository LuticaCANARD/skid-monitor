//! skid-monitor client.
//!
//! server agent가 보낸 신호를 수신하여 사용자에게 모니터링 정보를 보여주는 곳.

mod extension;
mod receiver;
mod view;

fn main() -> std::io::Result<()> {
    let addr = receiver::listen_addr();
    println!("skid-monitor client listening on {addr}");

    let mut extension_host = extension::ExtensionHost::from_env()?;
    let receiver = receiver::Receiver::bind(&addr)?;
    loop {
        match receiver.recv() {
            Ok(signal) => {
                view::render(&signal);
                if let Some(host) = extension_host.as_mut() {
                    if let Err(err) = host.publish_signal(&signal) {
                        eprintln!("failed to publish signal to extension host: {err}");
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                eprintln!("client connection closed before a complete signal arrived");
            }
            Err(err) => eprintln!("failed to receive signal: {err}"),
        }
    }
}
