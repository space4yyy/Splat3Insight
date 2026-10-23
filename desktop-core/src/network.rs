//! Shared HTTP proxy configuration for Nintendo and SplatNet requests.

use reqwest::{ClientBuilder, Proxy};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use url::{Host, Url};

/// Accept HTTP or HTTPS proxy endpoints without embedded credentials. The app
/// currently supports unauthenticated proxy servers only.
pub fn validate_proxy_address(address: &str) -> Result<String, String> {
    let address = address.trim();
    if address.is_empty() {
        return Err("请输入代理地址".to_owned());
    }

    let parsed = Url::parse(address).map_err(|_| "代理地址格式无效".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("代理地址必须以 http:// 或 https:// 开头".to_owned());
    }
    if parsed.host_str().is_none() {
        return Err("代理地址缺少主机名或 IP 地址".to_owned());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("代理地址中不能包含用户名或密码".to_owned());
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("代理地址只填写协议、主机名和端口，不要添加路径或参数".to_owned());
    }

    Ok(address.to_owned())
}

/// Apply an explicit app proxy while leaving reqwest's existing environment proxy
/// behavior intact when no app proxy has been configured.
pub fn apply_proxy(
    builder: ClientBuilder,
    proxy_address: Option<&str>,
) -> Result<ClientBuilder, String> {
    let Some(proxy_address) = proxy_address else {
        return Ok(builder);
    };
    let proxy_address = validate_proxy_address(proxy_address)?;
    let proxy = Proxy::all(&proxy_address).map_err(|_| "代理地址无法使用".to_owned())?;
    Ok(builder.proxy(proxy))
}

/// Check only whether the proxy host and port accept a TCP connection. This
/// does not send an HTTP request or contact the Nintendo service.
pub async fn test_proxy_connection(proxy_address: &str) -> Result<u16, String> {
    let address = validate_proxy_address(proxy_address)?;
    let parsed = Url::parse(&address).map_err(|_| "代理地址格式无效".to_owned())?;
    let host = parsed
        .host()
        .ok_or_else(|| "代理地址缺少主机名或 IP 地址".to_owned())?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "代理地址缺少有效端口".to_owned())?;
    let connect = async move {
        match host {
            Host::Domain(domain) => TcpStream::connect((domain, port)).await,
            Host::Ipv4(address) => {
                TcpStream::connect(std::net::SocketAddr::from((address, port))).await
            }
            Host::Ipv6(address) => {
                TcpStream::connect(std::net::SocketAddr::from((address, port))).await
            }
        }
    };

    match timeout(Duration::from_secs(5), connect).await {
        Ok(Ok(_stream)) => Ok(port),
        Ok(Err(_)) => Err("无法连接代理端口，请检查代理地址和端口".to_owned()),
        Err(_) => Err("连接代理端口超时，请检查代理是否正常运行".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https_proxy_addresses() {
        assert_eq!(
            validate_proxy_address(" http://127.0.0.1:7890 ").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            validate_proxy_address("https://proxy.example:8443/").unwrap(),
            "https://proxy.example:8443/"
        );
    }

    #[test]
    fn rejects_proxy_credentials_and_non_origin_urls() {
        for address in [
            "socks5://127.0.0.1:7890",
            "http://user:pass@127.0.0.1:7890",
            "http://127.0.0.1:7890/proxy",
            "http://127.0.0.1:7890/?token=value",
            "http://127.0.0.1:7890/#fragment",
        ] {
            assert!(validate_proxy_address(address).is_err());
        }
    }
}
