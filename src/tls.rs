//! TLS material loading shared by the future authenticated control and media relays.
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
};
use std::{
    fs::File,
    io::{self, BufReader},
    path::Path,
    sync::Arc,
};

pub fn load_server_config(
    certificate_path: &Path,
    key_path: &Path,
) -> io::Result<Arc<ServerConfig>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| io::Error::other("TLS crypto provider was already initialized differently"))
        .ok();
    let certificates: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(File::open(certificate_path)?))
            .collect::<Result<_, _>>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if certificates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TLS certificate file is empty",
        ));
    }
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut BufReader::new(
        File::open(key_path)?,
    ))?
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "TLS private key file is empty"))?;
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .map(Arc::new)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
