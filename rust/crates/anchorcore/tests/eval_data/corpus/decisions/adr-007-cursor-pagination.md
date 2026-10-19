# ADR-007: cursor pagination for list endpoints

Date: 2026-09-14. Status: accepted.

## Context

List endpoints return unbounded rows. Clients paginate these list
endpoints today with page numbers, which skips rows when new rows land
between requests.

## Decision

We choose cursor pagination for every list endpoint. Clients choose the
next page by passing back the opaque cursor the previous page returned.
Cursor pagination keeps each list endpoint stable under inserts, and the
required cursor parameter is enforced by the api-lint ruleset: any list
endpoint without a cursor parameter fails review.

## Consequences

Offset pagination is rejected: page numbers cannot name a stable row.
Every new endpoint ships cursor pagination from day one.
