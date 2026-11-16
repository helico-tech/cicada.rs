# Seat reservations

## Actors

- `shows/<show>` owns the seats of one show.

## Rules

- A seat is held for one customer at a time.
- A hold expires after 10 minutes unless the customer pays: a timer in the show releases the seat.
- Paying for a seat that is no longer held is refused.
