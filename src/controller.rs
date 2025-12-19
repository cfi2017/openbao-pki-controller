use std::{error::Error as _, sync::Arc, time::Duration};

use chrono::Utc;
use der::EncodePem;
use k8s_openapi::{
    api::certificates::v1alpha1::PodCertificateRequest,
    apimachinery::pkg::apis::meta::v1::{Condition, Time},
    serde_json::json,
};
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
    runtime::controller::Action,
};
use tracing::{debug, info, warn};

use crate::{error::Error, intermediate_ca::IntermediateCA};

pub struct Data {
    client: Client,
    ca: IntermediateCA,
}

impl Data {
    pub fn new(client: Client, ca: IntermediateCA) -> Self {
        Self { client, ca }
    }
}

/// Controller triggers this whenever our main object or our children changed
pub async fn reconcile(pcr: Arc<PodCertificateRequest>, ctx: Arc<Data>) -> Result<Action, Error> {
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
