# openbao-pki-controller

Quick and dirty PoC implementing a PodCertificateRequest signer using OpenBao as a backing PKI engine.

As PodCertificateRequests don't give us full CSRs and we can't sign leaf certificates with Bao directly, 
we instead issue a temporary intermediate CA with OpenBao that is held in memory by the controller and use that CA to sign leaf certificates.

