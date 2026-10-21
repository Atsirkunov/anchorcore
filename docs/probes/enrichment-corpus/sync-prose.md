Engineering sync, Oct 14. Present: Luis, Amara, Jon.

We spent most of the hour on the database question for the notifications
service. Amara walked through the write patterns and convinced everyone that
Postgres is the safer pick over MySQL here, mainly because of the JSONB
operators the digest queries need. Nobody objected, so we're going with
Postgres. Luis volunteered to run a quick benchmark on the digest query
before Friday just to confirm the numbers hold at peak volume.

Separately, Jon mentioned the Redis cluster upgrade is still waiting on the
vendor's maintenance window. Nothing to decide there until they reply.
