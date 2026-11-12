//! Finds server settings from just an email address.
//!
//! Follows the same order as Thunderbird's account setup
//! (`mail/components/accountcreation`), which has proven itself over many
//! years:
//!
//! 1. The provider's own autoconfig file (`autoconfig.<domain>` and
//!    `<domain>/.well-known/autoconfig`).
//! 2. Thunderbird's public provider database (ISPDB).
//! 3. The same two lookups for the domain of the MX server, which covers
//!    custom domains hosted at a big provider.
//! 4. RFC 6186 SRV records.
//! 5. Educated guesses (`imap.<domain>`, `mail.<domain>`, the MX host).
//!
//! Guesses are only candidates; callers must verify them by connecting (see
//! [`crate::imap::probe`]). Only HTTPS is used for fetching, since a
//! tampered config would send the password to an attacker's server.

use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;
use serde::Serialize;

use crate::model::{AuthMethod, Security, ServerConfig, split_address};
use crate::{Error, Result};

const ISPDB_URL: &str = "https://live.thunderbird.net/autoconfig/v1.1/";
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);

/// Where a candidate configuration came from, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Provider,
    Ispdb,
    MxProvider,
    MxIspdb,
    Srv,
    Guess,
}

impl Source {
    /// Whether the settings were published by someone who knows the
    /// server, as opposed to being guessed.
    pub fn is_authoritative(self) -> bool {
        !matches!(self, Source::Guess)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub source: Source,
    pub provider_name: Option<String>,
    pub imap: ServerConfig,
    pub smtp: Option<ServerConfig>,
}

/// Looks up all candidate configurations for `email`, best first.
pub async fn discover(email: &str) -> Result<Vec<Candidate>> {
    let (_, domain) = split_address(email)?;
    let domain = domain.to_ascii_lowercase();
    let http = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("Halcyon/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Other(e.to_string()))?;
    let resolver = TokioResolver::builder_tokio()
        .and_then(|b| b.build())
        .map_err(|e| Error::Other(format!("DNS resolver: {e}")))
        .ok();

    let (provider, ispdb, mx) = futures::join!(
        fetch_provider(&http, &domain, email),
        fetch_ispdb(&http, &domain, email),
        async {
            match &resolver {
                Some(r) => mx_host(r, &domain).await,
                None => None,
            }
        }
    );

    let mut found = Vec::new();
    found.extend(tag(provider, Source::Provider));
    found.extend(tag(ispdb, Source::Ispdb));

    if found.is_empty()
        && let Some(mx) = &mx
    {
        for mx_domain in mx_lookup_domains(mx, &domain) {
            let (p, i) = futures::join!(
                fetch_provider_https_only(&http, &mx_domain, email),
                fetch_ispdb(&http, &mx_domain, email)
            );
            found.extend(tag(p, Source::MxProvider));
            found.extend(tag(i, Source::MxIspdb));
            if !found.is_empty() {
                break;
            }
        }
    }

    if found.is_empty()
        && let Some(r) = &resolver
    {
        found.extend(srv_candidates(r, &domain, email).await);
    }

    if found.is_empty() {
        found.extend(guesses(&domain, email, mx.as_deref()));
    }

    found.sort_by_key(|c| (c.source, security_rank(c.imap.security)));
    found.dedup_by(|a, b| a.imap == b.imap && a.smtp == b.smtp);
    Ok(found)
}

fn tag(list: Option<Parsed>, source: Source) -> Vec<Candidate> {
    list.unwrap_or_default()
        .into_iter()
        .map(|(provider_name, imap, smtp)| Candidate {
            source,
            provider_name,
            imap,
            smtp,
        })
        .collect()
}

type Parsed = Vec<(Option<String>, ServerConfig, Option<ServerConfig>)>;

async fn fetch_provider(http: &reqwest::Client, domain: &str, email: &str) -> Option<Parsed> {
    let urls = [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ];
    for url in urls {
        if let Some(parsed) = fetch_xml(http, &url, email).await {
            return Some(parsed);
        }
    }
    None
}

async fn fetch_provider_https_only(
    http: &reqwest::Client,
    domain: &str,
    email: &str,
) -> Option<Parsed> {
    let url = format!("https://autoconfig.{domain}/mail/config-v1.1.xml");
    fetch_xml(http, &url, email).await
}

async fn fetch_ispdb(http: &reqwest::Client, domain: &str, email: &str) -> Option<Parsed> {
    fetch_xml(http, &format!("{ISPDB_URL}{domain}"), email).await
}

async fn fetch_xml(http: &reqwest::Client, url: &str, email: &str) -> Option<Parsed> {
    let response = http
        .get(url)
        .query(&[("emailaddress", email)])
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tracing::debug!(url, status = %response.status(), "no autoconfig");
        return None;
    }
    let body = response.text().await.ok()?;
    match parse_client_config(&body, email) {
        Ok(parsed) if !parsed.is_empty() => Some(parsed),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(url, error = %e, "unusable autoconfig");
            None
        }
    }
}

/// Parses Thunderbird's `clientConfig` XML format
/// (<https://wiki.mozilla.org/Thunderbird:Autoconfiguration:ConfigFileFormat>).
///
/// Returns one entry per usable IMAP server, each paired with the best SMTP
/// server. POP3, Exchange and unknown entries are skipped.
pub fn parse_client_config(xml: &str, email: &str) -> Result<Parsed> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Other(e.to_string()))?;
    let Some(provider) = doc.descendants().find(|n| n.has_tag_name("emailProvider")) else {
        return Ok(Vec::new());
    };
    let provider_name = child_text(provider, "displayName").map(str::to_string);

    let servers = |tag: &str, kind: &str| -> Vec<ServerConfig> {
        provider
            .children()
            .filter(|n| n.has_tag_name(tag) && n.attribute("type") == Some(kind))
            .flat_map(|n| servers_from_node(n, email))
            .collect()
    };

    let mut imap = servers("incomingServer", "imap");
    let mut smtp = servers("outgoingServer", "smtp");
    imap.sort_by_key(|s| security_rank(s.security));
    smtp.sort_by_key(|s| security_rank(s.security));

    // Pair each IMAP entry with the most secure SMTP server that uses the
    // same authentication, so one login method covers both.
    Ok(imap
        .into_iter()
        .map(|i| {
            let s = smtp
                .iter()
                .find(|s| s.auth == i.auth)
                .or(smtp.first())
                .cloned();
            (provider_name.clone(), i, s)
        })
        .collect())
}

/// One config per supported authentication method, in the server's order,
/// so callers can pick the method they can do (e.g. password before OAuth is
/// set up).
fn servers_from_node(node: roxmltree::Node<'_, '_>, email: &str) -> Vec<ServerConfig> {
    let Some(host) = child_text(node, "hostname").map(|h| substitute(h, email)) else {
        return Vec::new();
    };
    let Some(port) = child_text(node, "port").and_then(|p| p.parse().ok()) else {
        return Vec::new();
    };
    let security = match child_text(node, "socketType") {
        Some("SSL") => Security::Tls,
        Some("STARTTLS") => Security::StartTls,
        Some("plain") if is_local(&host) => Security::Plain,
        _ => return Vec::new(),
    };
    let username = substitute(
        child_text(node, "username").unwrap_or("%EMAILADDRESS%"),
        email,
    );
    let mut methods: Vec<AuthMethod> = node
        .children()
        .filter(|n| n.has_tag_name("authentication"))
        .filter_map(|n| n.text())
        .filter_map(|a| match a.trim() {
            "password-cleartext" | "plain" | "password-encrypted" | "secure" => {
                Some(AuthMethod::Password)
            }
            "OAuth2" => Some(AuthMethod::OAuth2),
            _ => None,
        })
        .collect();
    methods.dedup();
    methods
        .into_iter()
        .map(|auth| ServerConfig {
            host: host.clone(),
            port,
            security,
            username: username.clone(),
            auth,
        })
        .collect()
}

fn child_text<'a>(node: roxmltree::Node<'a, '_>, tag: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn substitute(template: &str, email: &str) -> String {
    let (local, domain) = split_address(email).unwrap_or((email, ""));
    template
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

fn security_rank(s: Security) -> u8 {
    match s {
        Security::Tls => 0,
        Security::StartTls => 1,
        Security::Plain => 2,
    }
}

fn is_local(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

async fn mx_host(resolver: &TokioResolver, domain: &str) -> Option<String> {
    let lookup = resolver.mx_lookup(domain).await.ok()?;
    lookup
        .answers()
        .iter()
        .filter_map(|r| match &r.data {
            RData::MX(mx) => Some((mx.preference, mx.exchange.to_utf8())),
            _ => None,
        })
        .min_by_key(|(pref, _)| *pref)
        .map(|(_, host)| host.trim_end_matches('.').to_ascii_lowercase())
}

/// Domains to look up for an MX host, most specific first:
/// `mx01.mail.example.com` → `mail.example.com`, `example.com`.
///
/// Without a public-suffix list this can produce a useless lookup for
/// domains like `co.uk`, which simply finds nothing.
fn mx_lookup_domains(mx: &str, own_domain: &str) -> Vec<String> {
    let labels: Vec<&str> = mx.split('.').collect();
    let mut out = Vec::new();
    for keep in [3, 2] {
        if labels.len() > keep {
            let d = labels[labels.len() - keep..].join(".");
            if d != own_domain && !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

async fn srv_candidates(resolver: &TokioResolver, domain: &str, email: &str) -> Vec<Candidate> {
    async fn srv(r: &TokioResolver, name: String) -> Option<(String, u16)> {
        let lookup = r.srv_lookup(name).await.ok()?;
        lookup
            .answers()
            .iter()
            .filter_map(|rec| match &rec.data {
                RData::SRV(s) => Some((s.priority, s.target.to_utf8(), s.port)),
                _ => None,
            })
            // A target of "." means "service not available" (RFC 2782).
            .filter(|(_, target, _)| target != ".")
            .min_by_key(|(prio, _, _)| *prio)
            .map(|(_, t, p)| (t.trim_end_matches('.').to_string(), p))
    }

    let (imaps, imap, submissions, submission) = futures::join!(
        srv(resolver, format!("_imaps._tcp.{domain}")),
        srv(resolver, format!("_imap._tcp.{domain}")),
        srv(resolver, format!("_submissions._tcp.{domain}")),
        srv(resolver, format!("_submission._tcp.{domain}")),
    );
    let server = |(host, port): (String, u16), security| ServerConfig {
        host,
        port,
        security,
        username: email.to_string(),
        auth: AuthMethod::Password,
    };
    let smtp = submissions
        .map(|s| server(s, Security::Tls))
        .or(submission.map(|s| server(s, Security::StartTls)));
    [
        imaps.map(|s| server(s, Security::Tls)),
        imap.map(|s| server(s, Security::StartTls)),
    ]
    .into_iter()
    .flatten()
    .map(|imap| Candidate {
        source: Source::Srv,
        provider_name: None,
        imap,
        smtp: smtp.clone(),
    })
    .collect()
}

fn guesses(domain: &str, email: &str, mx: Option<&str>) -> Vec<Candidate> {
    let mut hosts = vec![format!("imap.{domain}"), format!("mail.{domain}")];
    if let Some(mx) = mx {
        hosts.push(mx.to_string());
    }
    hosts.dedup();
    hosts
        .into_iter()
        .map(|host| {
            let smtp_host = host.replacen("imap.", "smtp.", 1);
            Candidate {
                source: Source::Guess,
                provider_name: None,
                imap: ServerConfig {
                    host,
                    port: 993,
                    security: Security::Tls,
                    username: email.to_string(),
                    auth: AuthMethod::Password,
                },
                smtp: Some(ServerConfig {
                    host: smtp_host,
                    port: 465,
                    security: Security::Tls,
                    username: email.to_string(),
                    auth: AuthMethod::Password,
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.org">
    <domain>example.org</domain>
    <displayName>Example Mail</displayName>
    <incomingServer type="pop3">
      <hostname>pop.example.org</hostname><port>995</port>
      <socketType>SSL</socketType><authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>imap.example.org</hostname><port>143</port>
      <socketType>STARTTLS</socketType><authentication>password-cleartext</authentication>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>imap.example.org</hostname><port>993</port>
      <socketType>SSL</socketType>
      <authentication>OAuth2</authentication>
      <authentication>password-cleartext</authentication>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>legacy.example.org</hostname><port>143</port>
      <socketType>plain</socketType><authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.org</hostname><port>587</port>
      <socketType>STARTTLS</socketType><authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.org</hostname><port>465</port>
      <socketType>SSL</socketType><authentication>NTLM</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn parses_imap_servers_best_first() {
        let parsed = parse_client_config(SAMPLE, "max@example.org").unwrap();
        // POP3 and the unencrypted remote server are dropped; the SSL
        // server yields one entry per auth method.
        assert_eq!(parsed.len(), 3);
        let (name, imap, smtp) = &parsed[0];
        assert_eq!(name.as_deref(), Some("Example Mail"));
        assert_eq!(imap.port, 993);
        assert_eq!(imap.security, Security::Tls);
        // Methods keep the server's order.
        assert_eq!(imap.auth, AuthMethod::OAuth2);
        assert_eq!(parsed[1].1.auth, AuthMethod::Password);
        assert_eq!(imap.username, "max");
        // The SSL SMTP entry only offers NTLM, so STARTTLS on 587 is chosen.
        let smtp = smtp.as_ref().unwrap();
        assert_eq!(smtp.port, 587);
        assert_eq!(smtp.username, "max@example.org");
        assert_eq!(parsed[2].1.security, Security::StartTls);
    }

    #[test]
    fn rejects_broken_xml() {
        assert!(parse_client_config("<clientConfig", "a@b.de").is_err());
        assert!(parse_client_config("<html/>", "a@b.de").unwrap().is_empty());
    }

    #[test]
    fn mx_domains_skip_own_domain() {
        assert_eq!(
            mx_lookup_domains("aspmx.l.google.com", "firma.de"),
            vec!["l.google.com", "google.com"]
        );
        assert_eq!(
            mx_lookup_domains("mx.firma.de", "firma.de"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn guesses_include_mx_host() {
        let g = guesses("example.org", "a@example.org", Some("mx2.hoster.net"));
        let hosts: Vec<_> = g.iter().map(|c| c.imap.host.as_str()).collect();
        assert_eq!(
            hosts,
            ["imap.example.org", "mail.example.org", "mx2.hoster.net"]
        );
        assert_eq!(g[0].smtp.as_ref().unwrap().host, "smtp.example.org");
    }
}
