use std::{
    env,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    process::ExitCode,
    time::Duration,
};

const HELP: &str =
    "TDS Factory\n\nA small software factory.\n\nUsage: tds-factory [dashboard | --help | --version]";
const DASHBOARD: &str = include_str!("dashboard.html");
const DASHBOARD_ADDRESS: &str = "127.0.0.1:4311";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => println!("{HELP}"),
        [arg] if arg == "--help" || arg == "-h" => println!("{HELP}"),
        [arg] if arg == "--version" || arg == "-V" => {
            println!("tds-factory {}", env!("CARGO_PKG_VERSION"));
        }
        [arg] if arg == "dashboard" => {
            if let Err(error) = serve_dashboard() {
                eprintln!("Dashboard error: {error}");
                return ExitCode::FAILURE;
            }
        }
        _ => {
            eprintln!("Unknown arguments. Run tds-factory --help for usage.");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn serve_dashboard() -> io::Result<()> {
    let listener = TcpListener::bind(DASHBOARD_ADDRESS).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not listen on {DASHBOARD_ADDRESS}: {error}"),
        )
    })?;
    println!("Dashboard: http://{DASHBOARD_ADDRESS} (press Ctrl+C to stop)");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(error) = respond(stream) {
                    eprintln!("Dashboard request error: {error}");
                }
            }
            Err(error) => eprintln!("Dashboard connection error: {error}"),
        }
    }
    Ok(())
}

fn respond(mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut request = [0; 4096];
    let mut count = 0;
    while count < request.len() && !request[..count].contains(&b'\n') {
        let read = stream.read(&mut request[count..])?;
        if read == 0 {
            break;
        }
        count += read;
    }
    let (status, content_type, body) = response_for(&request[..count]);
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

fn response_for(request: &[u8]) -> (&'static str, &'static str, &'static str) {
    let Some(line_end) = request.iter().position(|byte| *byte == b'\n') else {
        return (
            "400 Bad Request",
            "text/plain; charset=utf-8",
            "Bad request",
        );
    };
    let line = request[..line_end]
        .strip_suffix(b"\r")
        .unwrap_or(&request[..line_end]);
    let mut parts = line.split(|byte| *byte == b' ');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(b"GET"), Some(b"/"), Some(b"HTTP/1.1" | b"HTTP/1.0"), None) => {
            ("200 OK", "text/html; charset=utf-8", DASHBOARD)
        }
        (Some(b"GET"), Some(path), Some(b"HTTP/1.1" | b"HTTP/1.0"), None)
            if path.starts_with(b"/") =>
        {
            ("404 Not Found", "text/plain; charset=utf-8", "Not found")
        }
        (Some(method), Some(path), Some(b"HTTP/1.1" | b"HTTP/1.0"), None)
            if !method.is_empty() && path.starts_with(b"/") =>
        {
            (
                "405 Method Not Allowed",
                "text/plain; charset=utf-8",
                "Method not allowed",
            )
        }
        _ => (
            "400 Bad Request",
            "text/plain; charset=utf-8",
            "Bad request",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::response_for;

    #[test]
    fn routes_dashboard_requests_and_rejects_invalid_requests() {
        assert_eq!(response_for(b"GET / HTTP/1.1\r\n").0, "200 OK");
        assert_eq!(
            response_for(b"GET /missing HTTP/1.1\r\n").0,
            "404 Not Found"
        );
        assert_eq!(
            response_for(b"POST / HTTP/1.1\r\n").0,
            "405 Method Not Allowed"
        );
        assert_eq!(response_for(b"GET / HTTP/1.1").0, "400 Bad Request");
        assert_eq!(response_for(b"GET /\r\n").0, "400 Bad Request");
    }
}
