Review of the Oct 9 checkout outage. Attendees: Priya, Dev, on-call lead.

The outage started when the deploy removed a feature flag the mobile client
still reads. Priya showed the timeline: flag removed at 14:02, first errors
at 14:05, rollback finished 14:31. Total impact was 26 minutes of elevated
errors on iOS checkout.

We agreed on two follow-ups. First, Dev will add a client-compatibility
check to the deploy checklist so flag removals get a second look. Second,
Priya will write up the mobile version matrix somewhere the backend team
can find it, because nobody knew which app versions still read the flag.
No blame discussion; the process gap was the point.
