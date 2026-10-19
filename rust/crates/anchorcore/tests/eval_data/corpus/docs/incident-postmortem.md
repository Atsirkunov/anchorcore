# Postmortem: checkout outage (INC-5521)

On October 3rd the checkout flow went down for 47 minutes. The checkout
outage was caused by an expired certificate on the payments gateway:
every checkout attempt failed with error code ERR-90412 at the gateway
edge, and the on-call engineer was paged by the gateway alert fifteen
minutes after the first failure. The alert delay was caused by a
threshold gap: the alert counted only 5xx responses while the gateway
returned 4xx.

## Timeline

- 14:02 — first failed checkout, outage begins.
- 14:17 — gateway alert pages the on-call engineer.
- 14:31 — cause identified: expired gateway certificate.
- 14:49 — certificate renewed, outage ends, checkout flow recovers.

## Follow-up

The main follow-up action is a certificate-expiry monitor with a
thirty-day warning. A second follow-up item backfills the missing
checkout orders and extends the alert rules to cover gateway 4xx
responses so the next outage pages within two minutes.
