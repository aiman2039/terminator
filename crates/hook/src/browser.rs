//! Explicit control of an already-running external browser. No browser engine
//! is embedded, downloaded, or automatically started with a debugging endpoint.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use clap::Subcommand;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr, TcpStream},
    path::PathBuf,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, protocol::WebSocketConfig};
#[derive(Subcommand)]
pub enum BrowserCommand {
    Open {
        url: String,
    },
    Navigate {
        #[arg(long)]
        endpoint: String,
        url: String,
    },
    Snapshot {
        #[arg(long)]
        endpoint: String,
    },
    Screenshot {
        #[arg(long)]
        endpoint: String,
        output: PathBuf,
    },
    Click {
        #[arg(long)]
        endpoint: String,
        selector: String,
    },
    Fill {
        #[arg(long)]
        endpoint: String,
        selector: String,
        text: String,
    },
    Evaluate {
        #[arg(long)]
        endpoint: String,
        expression: String,
    },
}
fn endpoint(value: &str) -> Result<(url::Url, SocketAddr)> {
    let url = url::Url::parse(value).context("Invalid CDP WebSocket URL")?;
    ensure!(
        url.scheme() == "ws" && url.username().is_empty() && url.password().is_none(),
        "Use an explicit local ws:// page debugging endpoint"
    );
    let host = url.host_str().context("CDP host missing")?;
    let ip: IpAddr = if host == "localhost" {
        IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    } else {
        host.trim_matches(['[', ']'])
            .parse()
            .context("CDP host must be a loopback address")?
    };
    ensure!(
        ip.is_loopback(),
        "CDP endpoint must be local; use an explicit SSH tunnel for remote browsers"
    );
    let port = url.port().context("CDP port required")?;
    Ok((url, SocketAddr::new(ip, port)))
}
struct Client {
    socket: WebSocket<TcpStream>,
    sequence: u64,
    events: std::collections::VecDeque<Value>,
}
impl Client {
    fn connect(value: &str) -> Result<Self> {
        let (url, address) = endpoint(value)?;
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let (socket, _) = tungstenite::client::client_with_config(
            url.as_str(),
            stream,
            Some(
                WebSocketConfig::default()
                    .max_message_size(Some(16 * 1024 * 1024))
                    .max_frame_size(Some(16 * 1024 * 1024)),
            ),
        )?;
        Ok(Self {
            socket,
            sequence: 0,
            events: std::collections::VecDeque::default(),
        })
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.sequence = self.sequence.saturating_add(1);
        let id = self.sequence;
        self.socket.send(Message::Text(
            json!({"id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))?;
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(5))
            .unwrap_or_else(Instant::now);
        for _ in 0..512 {
            ensure!(Instant::now() < deadline, "CDP response timed out");
            if let Message::Text(text) = self.socket.read()? {
                let value: Value = serde_json::from_str(&text)?;
                if value.get("id").is_some_and(|found| *found == id) {
                    if let Some(error) = value.get("error") {
                        anyhow::bail!("Browser rejected command: {error}");
                    }
                    return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                }
                if self.events.len() == 64 {
                    self.events.pop_front();
                }
                self.events.push_back(value);
            }
        }
        anyhow::bail!("Too many browser events without a response")
    }
    fn navigate(&mut self, url: String) -> Result<Value> {
        self.call("Page.enable", json!({}))?;
        self.events.clear();
        let result = self.call("Page.navigate", json!({"url":url}))?;
        if let Some(error) = result.get("errorText") {
            anyhow::bail!("Navigation failed: {error}");
        }
        if result.get("loaderId").is_some() {
            let deadline = Instant::now()
                .checked_add(Duration::from_secs(10))
                .unwrap_or_else(Instant::now);
            loop {
                if self.events.iter().any(|event| {
                    event
                        .get("method")
                        .is_some_and(|method| method == "Page.loadEventFired")
                }) {
                    break;
                }
                ensure!(Instant::now() < deadline, "Page load timed out");
                if let Message::Text(text) = self.socket.read()? {
                    let event: Value = serde_json::from_str(&text)?;
                    if event
                        .get("method")
                        .is_some_and(|method| method == "Page.loadEventFired")
                    {
                        break;
                    }
                }
            }
        }
        Ok(result)
    }
    fn evaluate(&mut self, expression: String) -> Result<Value> {
        let result = self.call(
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true,"awaitPromise":true}),
        )?;
        if let Some(error) = result.get("exceptionDetails") {
            anyhow::bail!("Browser script failed: {error}");
        }
        Ok(result
            .get("result")
            .and_then(|inner| inner.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }
}
fn selector_expression(selector: &str, text: Option<&str>) -> Result<String> {
    let selector = serde_json::to_string(selector)?;
    let action = if let Some(text) = text {
        let literal = serde_json::to_string(text)?;
        format!(
            "const v={literal}; if(e.isContentEditable) e.textContent=v; const p=Object.getPrototypeOf(e); const setter=Object.getOwnPropertyDescriptor(p,'value')?.set; if(setter) setter.call(e,v); else e.value=v; e.dispatchEvent(new Event('input',{{bubbles:true}})); e.dispatchEvent(new Event('change',{{bubbles:true}}));"
        )
    } else {
        "e.click();".into()
    };
    Ok(format!(
        "(()=>{{const e=document.querySelector({selector});if(!e)throw new Error('Element not found');{action}return true;}})()"
    ))
}
pub fn run(command: BrowserCommand) -> Result<()> {
    let value=match command {
        BrowserCommand::Open {url}=>{open::that(terminator_core::metadata::http_url(&url)?)?;json!({"opened":true})},
        BrowserCommand::Navigate {endpoint,url}=>Client::connect(&endpoint)?.navigate(terminator_core::metadata::http_url(&url)?)?,
        BrowserCommand::Snapshot {endpoint}=>Client::connect(&endpoint)?.evaluate("({url:location.href,title:document.title,html:document.documentElement.outerHTML,styles:[...document.styleSheets].slice(0,64).map(s=>{try{return {href:s.href,text:[...s.cssRules].map(r=>r.cssText).join('\\n').slice(0,65536)}}catch{return {href:s.href,text:null}}})})".into())?,
        BrowserCommand::Screenshot {endpoint,output}=>{
            let response=Client::connect(&endpoint)?.call("Page.captureScreenshot",json!({"format":"png","captureBeyondViewport":false}))?;
            let bytes=B64.decode(response.get("data").and_then(Value::as_str).context("Screenshot missing")?)?;
            if let Some(parent)=output.parent().filter(|p|!p.as_os_str().is_empty()) {std::fs::create_dir_all(parent)?;}
            std::fs::write(&output,bytes)?;json!({"screenshot":output})
        }
        BrowserCommand::Click {endpoint,selector}=>Client::connect(&endpoint)?.evaluate(selector_expression(&selector,None)?)?,
        BrowserCommand::Fill {endpoint,selector,text}=>Client::connect(&endpoint)?.evaluate(selector_expression(&selector,Some(&text))?)?,
        BrowserCommand::Evaluate {endpoint,expression}=>Client::connect(&endpoint)?.evaluate(expression)?,
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selectors_and_values_are_serialized_as_literals() {
        let selector = "input[data-name=\"a'b\"]";
        let value = "';window.bad=true;//";
        let expression = selector_expression(selector, Some(value)).unwrap();
        assert!(expression.contains(&serde_json::to_string(selector).unwrap()));
        assert!(expression.contains(&serde_json::to_string(value).unwrap()));
    }
    #[test]
    fn debugging_requires_an_explicit_local_endpoint() {
        assert!(endpoint("ws://127.0.0.1:9222/devtools/page/fixture").is_ok());
        for url in [
            "ws://example.com:9222/",
            "http://localhost:9222/",
            "ws://user:pass@localhost:9222/",
        ] {
            assert!(endpoint(url).is_err());
        }
    }
}
