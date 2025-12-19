use std::time::SystemTime;

use rcgen::KeyPair;
use vaultrs::api::pki::responses::SignIntermediateResponse;

/// CACertificate represents a CA certificate
pub(crate) struct CACertificate {
    pub(crate) certificate_pem: String,
    pub(crate) key_pair: KeyPair,
}

impl From<(KeyPair, SignIntermediateResponse)> for CACertificate {
    fn from(value: (KeyPair, SignIntermediateResponse)) -> Self {
        Self {
            certificate_pem: value.1.certificate,
            key_pair: value.0,
        }
    }
}

impl CACertificate {
    pub fn is_expired(&self) -> bool {
        use der::DecodePem;
        let cert = match x509_cert::Certificate::from_pem(&self.certificate_pem) {
            Ok(cert) => cert,
            Err(_) => return true,
        };

        cert.tbs_certificate.validity.not_after.to_system_time() < SystemTime::now()
    }
}
