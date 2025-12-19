use k8s_openapi::api::certificates::v1alpha1::PodCertificateRequest;
use std::sync::Arc;
use tokio::sync::RwLock;

use rcgen::{CertificateParams, KeyPair};
use tracing::{debug, info, warn};
use vaultrs::{api::pki::requests::SignIntermediateRequestBuilder, client::VaultClient};

use crate::{Error, ca_certificate::CACertificate, utils::sign_certificate};

/// IntermediateCA is a client that implements managing its own in-memory CA based on an OpenBao
/// root CA. it performs no actions on initialisation, instead it generates its CA once the first
/// leaf certificate is consumed. refreshing the CA certificate is not currently supported.
pub(crate) struct IntermediateCA {
    bao: VaultClient,
    ca: Arc<RwLock<Option<CACertificate>>>,
}

impl IntermediateCA {
    pub fn new(client: VaultClient) -> Self {
        Self {
            bao: client,
            ca: Arc::new(RwLock::new(None)),
        }
    }

    async fn issue_ca_certificate(&self) -> Result<(), Error> {
        debug!("generating CA KeyPair");
        let ca_key_pair = KeyPair::generate().map_err(|e| {
            warn!("Failed to generate CA keypair: {:?}", e);
            Error::CSRCreate(e)
        })?;

        let common_name = gethostname::gethostname().into_string().unwrap();
        let mut params = CertificateParams::new(vec![common_name.to_string()]).map_err(|e| {
            warn!("Failed to create certificate params: {:?}", e);
            Error::CSRCreate(e)
        })?;

        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, &common_name);

        let csr = params.serialize_request(&ca_key_pair).map_err(|e| {
            warn!("Failed to serialize CSR: {:?}", e);
            Error::CSRCreate(e)
        })?;

        let csr_pem = csr.pem().map_err(|e| {
            warn!("Failed to encode CSR to PEM: {:?}", e);
            Error::CSRCreate(e)
        })?;

        let mut request_options = SignIntermediateRequestBuilder::default();
        request_options.ttl("168h");

        let intermediate = vaultrs::pki::cert::ca::sign_intermediate(
            &self.bao,
            "pki",
            &csr_pem,
            &common_name,
            Some(&mut request_options),
        )
        .await
        .map_err(|e| {
            warn!("Vault failed to sign intermediate CA: {:?}", e);
            Error::VaultRequestFailed(e)
        })?;

        info!("Intermediate CA certificate issued from Vault");

        // replace own state
        self.ca
            .to_owned()
            .write_owned()
            .await
            .replace((ca_key_pair, intermediate).into());

        Ok(())
    }

    pub async fn sign_certificate(
        &self,
        request: &PodCertificateRequest,
    ) -> Result<x509_cert::Certificate, Error> {
        if self.ca.read().await.is_none() {
            info!("issuing intermediate CA certificate");
            self.issue_ca_certificate().await?;
        }

        if self
            .ca
            .read()
            .await
            .as_ref()
            .is_some_and(|cert| cert.is_expired())
        {
            // renew certificate
            info!("renewing intermediate CA certificate");
            todo!();
        }

        // sign leaf certificate
        let public_key = &request.spec.pkix_public_key;

        let ca = self.ca.read().await;
        let ca_cert = ca.as_ref().unwrap();

        let cn = format!(
            "system:pod:{}:{}",
            request.metadata.namespace.as_deref().unwrap_or("default"),
            request.spec.pod_name
        );

        sign_certificate(public_key, &ca_cert.certificate_pem, &ca_cert.key_pair, &cn)
    }
}
