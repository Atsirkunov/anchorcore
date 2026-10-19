# ADR-012: billing provider selection

Date: 2026-08-30. Status: accepted.

## Context

The billing stack needs a provider that handles invoicing, dunning,
and tax receipts. Three billing providers were evaluated against the
scorecard: ledger accuracy, invoice flexibility, and support quality.

## Decision

Northwind Billing was selected as the billing provider. The provider
was selected for its ledger-correct invoices and its dunning flows.
Each invoice carries line items the ledger team reconciles daily.

## Consequences

The roadmap migrates invoicing to the selected provider this quarter.
Dunning templates move next. Billing support trains on provider tools.
