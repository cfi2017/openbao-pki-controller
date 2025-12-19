# openbao-pki-controller

Quick and dirty PoC implementing a PodCertificateRequest signer using OpenBao as a backing PKI engine.

As PodCertificateRequests don't give us full CSRs and we can't sign leaf certificates with Bao directly, 
we instead issue a temporary intermediate CA with OpenBao that is held in memory by the controller and use that CA to sign leaf certificates.

A production ready implementation should probably
- support Kubernetes auth so you can run the controller from inside a cluster
- implement intermediate CA rotation
- review certificate expiry, we currently make some very dumb assumptions
- probably provide a helm chart of some kind

## References

- https://kubernetes.io/docs/reference/access-authn-authz/certificate-signing-requests/#pod-certificate-requests
- https://github.com/kubernetes/enhancements/tree/master/keps/sig-auth/4317-pod-certificates

## Examples

- A pod.yaml is provided
