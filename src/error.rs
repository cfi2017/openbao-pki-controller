use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to create ConfigMap: {0}")]
    ConfigMapCreationFailed(#[source] kube::Error),
    #[error("MissingObjectKey: {0}")]
    MissingObjectKey(&'static str),
    #[error("Vault request failed: {0}")]
    VaultRequestFailed(#[source] vaultrs::error::ClientError),
    #[error("CSR creation failed: {0}")]
    CSRCreate(#[source] rcgen::Error),
    #[error("DER encoding failed: {0}")]
    Der(#[source] der::Error),
    #[error("Failed to sign certificate: {0}")]
    Signing(String),
}
