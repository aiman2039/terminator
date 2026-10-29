# vnc-rs

Vendored vnc-rs 0.6.0 (MIT OR Apache-2.0), from the registry package.
The original license notices are retained.

Local changes: strict authentication selection, certificate-validated VeNCrypt
X509Vnc/X509Plain with rustls, bounded clipboard and resize controls, and
quality/compression encoding presets, and server versions above 3.8 (RealVNC
`RFB 005.000`) negotiating 3.8 instead of 3.3. Plain transport is explicit and never
a fallback from a failed TLS or SSH connection.
