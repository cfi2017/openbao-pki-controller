use std::{env, error::Error as StdError, io::BufRead, sync::Arc, time::Duration};

use anyhow::Context;
use chrono::Utc;
use der::EncodePem;
use futures::StreamExt;
use k8s_openapi::{
    api::certificates::v1alpha1::PodCertificateRequest,
    apimachinery::pkg::apis::meta::v1::{Condition, Time},
    serde_json::json,
};
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
    runtime::{Config, Controller, controller::Action, watcher},
};
use thiserror::Error;
use tracing::{debug, info, warn};
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};

use crate::intermediate_ca::IntermediateCA;

mod ca_certificate;
mod intermediate_ca;
mod utils;

#[derive(Debug, Error)]
enum Error {
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

/// Controller triggers this whenever our main object or our children changed
async fn reconcile(pcr: Arc<PodCertificateRequest>, ctx: Arc<Data>) -> Result<Action, Error> {
    let client = &ctx.client;
    let pcr_name = pcr.metadata.name.as_deref().unwrap_or("unknown");
    let pod_uid = &pcr.spec.pod_uid;

    debug!("Reconciling PCR {} for pod {}", pcr_name, pod_uid);

    if let Some(status) = &pcr.status
        && status.certificate_chain.is_some()
    {
        // certificate is issued, break early
        debug!("Certificate already issued for PCR {}", pcr_name);
        return Ok(Action::requeue(Duration::from_secs(300)));
    }

    info!(
        "Issuing certificate for pod {} (PCR: {})",
        pod_uid, pcr_name
    );
    let cert = ctx.ca.sign_certificate(&pcr).await.map_err(|e| {
        warn!("Failed to sign certificate for PCR {}: {:?}", pcr_name, e);
        let mut source = e.source();
        while let Some(err) = source {
            warn!("  caused by: {}", err);
            source = err.source();
        }
        e
    })?;

    debug!("Certificate signed successfully for PCR {}", pcr_name);

    let not_before = cert.tbs_certificate.validity.not_before.to_system_time();
    let not_after = cert.tbs_certificate.validity.not_after.to_system_time();

    let renew_at = chrono::Utc::now()
        + Duration::from_secs(pcr.spec.max_expiration_seconds.unwrap() as u64)
        - Duration::from_secs(3600);

    let mut status = pcr.status.to_owned().unwrap_or_default().clone();
    status.certificate_chain = Some(cert.to_pem(der::pem::LineEnding::LF).map_err(|e| {
        warn!(
            "Failed to encode certificate to PEM for PCR {}: {:?}",
            pcr_name, e
        );
        Error::Der(e)
    })?);
    status.not_before = Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
        not_before.into(),
    ));
    status.not_after = Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
        not_after.into(),
    ));
    status.begin_refresh_at = Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
        renew_at,
    ));

    status.conditions = Some(vec![Condition {
        last_transition_time: Time(Utc::now()),
        message: String::from("Certificate issued successfully"),
        observed_generation: None,
        reason: String::from("CertificateIssuedSuccessfully"),
        status: String::from("True"),
        type_: String::from("Issued"),
    }]);

    let pcrs = Api::<PodCertificateRequest>::namespaced(
        client.clone(),
        pcr.metadata
            .namespace
            .as_ref()
            .ok_or_else(|| Error::MissingObjectKey(".metadata.namespace"))?,
    );
    debug!(
        "Patching status for PodCertificateRequest {} (pod {})",
        pcr_name, pod_uid
    );
    pcrs.patch_status(
        pcr.metadata.name.as_ref().unwrap(),
        &PatchParams::default(),
        &Patch::Merge(json!({"status": &status})),
    )
    .await
    .map_err(|e| {
        warn!("Failed to patch status for PCR {}: {:?}", pcr_name, e);
        Error::ConfigMapCreationFailed(e)
    })?;

    info!(
        "Successfully issued certificate for pod {} (PCR: {})",
        pod_uid, pcr_name
    );
    Ok(Action::requeue(Duration::from_secs(300)))
}

fn error_policy(object: Arc<PodCertificateRequest>, error: &Error, _ctx: Arc<Data>) -> Action {
    let pcr_name = object.metadata.name.as_deref().unwrap_or("unknown");
    warn!("Reconciliation error for PCR {}: {}", pcr_name, error);

    Action::requeue(Duration::from_secs(5))
}

struct Data {
    client: Client,
    ca: IntermediateCA,
}

// code mostly taken from the kube.rs example
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let client = Client::try_default().await?;

    let pcrs = Api::<PodCertificateRequest>::all(client.clone());

    info!("starting openbao-pki-controller");
    info!("press <enter> to force a reconciliation of all objects");

    let (mut reload_tx, reload_rx) = futures::channel::mpsc::channel(0);
    // Using a regular background thread since tokio::io::stdin() doesn't allow aborting reads,
    // and its worker prevents the Tokio runtime from shutting down.
    std::thread::spawn(move || {
        for _ in std::io::BufReader::new(std::io::stdin()).lines() {
            let _ = reload_tx.try_send(());
        }
    });

    let mut settings = VaultClientSettingsBuilder::default();

    settings.address(env::var("BAO_ADDR").context("Please set BAO_ADDR")?);

    if let Ok(token) = env::var("BAO_TOKEN") {
        settings.token(token);
    } else {
        // TODO: implement kubernetes authentication
    }

    let settings = settings.build()?;
    let bao = VaultClient::new(settings)?;

    let hostname = "";
    let namespace = "";
    let cluster_domain = "";
    let _common_name = format!("{}.{}.svc.{}", hostname, namespace, cluster_domain);

    // limit the controller to running a maximum of two concurrent reconciliations
    let config = Config::default().concurrency(2);

    let ca = IntermediateCA::new(bao);

    Controller::new(pcrs, watcher::Config::default())
        .with_config(config)
        .reconcile_all_on(reload_rx.map(|_| ()))
        .shutdown_on_signal()
        .run(reconcile, error_policy, Arc::new(Data { client, ca }))
        .for_each(|res| async move {
            match res {
                Ok(o) => info!("reconciled {:?}", o),
                Err(e) => warn!("reconcile failed: {}", e),
            }
        })
        .await;
    info!("controller terminated");
    Ok(())
}
