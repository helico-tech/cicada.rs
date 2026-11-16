# Cart and stock

One customer has one cart. Stock is shared by every customer.

## Requirements

- A cart never shows an item as reserved unless its stock actor has reserved it.
- Stock for one SKU is never reserved beyond its available count.
- A reservation that is not checked out within 15 minutes is released.
- A reply from stock that arrives after the cart moved on changes nothing.

## Actors

- `carts/<customer>` owns the cart lines and the checkout of that cart.
- `stock/<sku>` owns the available count and the reservations of one SKU.

## Decisions

- Cart and checkout are one actor, not two. One customer's cart changes and checkout serialize; we accept that,
  because a customer has one cart and checkout is rare compared with browsing. A separate checkout actor would need
  a saga for "lines changed during checkout" that we do not want.
- Reserve is a request/reply: `Reserve { request }` gets `Reserved { request }` or `Refused { request }`. The cart
  keeps the request id of each pending line and ignores replies whose request it does not hold.
- The cart arms one expiry timer per reservation and sends `Release { request }` when it fires. Checkout sends
  `Confirm { request }` for each line in the commit that marks the cart checked out; stock then forgets the entry
  and keeps the count taken. A checked-out cart is final; how the customer's next cart starts (the host deletes the
  finished one) is out of scope here.
