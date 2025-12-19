use std::{str::FromStr, time::Duration};

use der::{Decode, DecodePem};
use ecdsa::SigningKey;
use k8s_openapi::ByteString;
use p256::NistP256;
use pkcs8::DecodePrivateKey;
use rcgen::KeyPair;
use spki::SubjectPublicKeyInfoOwned;
use tracing::{debug, warn};
use x509_cert::{
    builder::{Builder, CertificateBuilder, Profile},
    name::Name,
    serial_number::SerialNumber,
    time::Validity,
};

use crate::Error;

/// sign_certificate signs a certificate for a given pubkey using an intermediate CA certificate
pub fn sign_certificate(
    pubkey: &ByteString,
    ca_cert_pem: &str,
    ca_keypair: &KeyPair,
    cn: &str,
) -> Result<x509_cert::Certificate, Error> {
    debug!("Signing certificate for CN={}", cn);

    let subject_public_key =
        SubjectPublicKeyInfoOwned::from_der(pubkey.0.as_slice()).map_err(|e| {
            warn!("Failed to parse public key as SPKI: {:?}", e);
            Error::Der(e)
        })?;

    let ca_cert = x509_cert::Certificate::from_pem(ca_cert_pem).map_err(|e| {
        warn!("Failed to parse CA certificate from PEM: {:?}", e);
        Error::Der(e)
    })?;
    let issuer_name = ca_cert.tbs_certificate.subject.clone();

    let cn_formatted = if cn.is_empty() {
        "CN=pod-certificate".to_string()
    } else {
        format!("CN={}", cn)
    };
    let subject = Name::from_str(&cn_formatted)
        .unwrap_or_else(|_| Name::from_str("CN=pod-certificate").unwrap());
    let serial_number = SerialNumber::from(u64::from_be_bytes(rand::random::<[u8; 8]>()));
    let validity = Validity::from_now(Duration::from_secs(86400)).map_err(|e| {
        warn!("Failed to create validity period: {:?}", e);
        Error::Der(e)
    })?;
    let ca_key_der = ca_keypair.serialize_der();
    let signing_key = SigningKey::<NistP256>::from_pkcs8_der(&ca_key_der).map_err(|e| {
        warn!("Failed to convert CA keypair to ECDSA signing key: {:?}", e);
        Error::Signing(format!("Key conversion failed: {}", e))
    })?;

    let builder = CertificateBuilder::new(
        Profile::Leaf {
            issuer: issuer_name,
            enable_key_agreement: false,
            enable_key_encipherment: false,
        },
        serial_number,
        validity,
        subject,
        subject_public_key,
        &signing_key,
    )
    .map_err(|e| {
        warn!("Failed to create certificate builder: {:?}", e);
        Error::Signing(format!("Certificate builder creation failed: {}", e))
    })?;

    let cert = builder.build::<p256::ecdsa::DerSignature>().map_err(|e| {
        warn!("Failed to sign certificate: {:?}", e);
        Error::Signing(format!("Certificate signing failed: {}", e))
    })?;

    Ok(cert)
}
