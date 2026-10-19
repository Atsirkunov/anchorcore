# Production deploy runbook

This runbook covers every production deploy of the storefront service.

## Rollout

1. Announce the deploy window in the team channel.
2. Promote the canary build to ten percent of traffic.
3. Watch error budget burn for thirty minutes.
4. Promote the build to full production traffic.

## Roll back

When a production deploy misbehaves, roll back before debugging.
To roll back, flip the `legacy_checkout` feature flag to route traffic
to the previous build, then redeploy the last known-good image.
Every roll back is logged with the deploy id and the reason.
Practice the roll back path in preprod each month so the real one is boring.
