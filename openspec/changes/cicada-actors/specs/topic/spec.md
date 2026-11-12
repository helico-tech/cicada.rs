# topic

## ADDED Requirements

### Requirement: A topic sends each published message to its subscribers

A topic SHALL have the type name `cicada.topic`. `Subscribe` SHALL add an address to its subscribers, and
`Unsubscribe` remove it. `Publish` SHALL lead to one message for each subscriber, with the type name and the
payload that were published, by a `Deliver` that the topic sends itself and that sends only to a current
subscriber. One subscriber SHALL get the messages in the order they were published, but for a retried delivery.

#### Scenario: Two subscribers

- **WHEN** two actors subscribed to a topic and a message is published
- **THEN** each has that message in its mailbox once

#### Scenario: The order for one subscriber

- **WHEN** three messages are published to a topic
- **THEN** a subscriber gets them in that order

#### Scenario: A subscriber leaves

- **WHEN** an actor is unsubscribed and a message is then published
- **THEN** it does not get the message

#### Scenario: A publish before an unsubscribe

- **WHEN** a message is published and the subscriber is unsubscribed before its delivery is handled
- **THEN** the subscriber does not get the message

#### Scenario: A delivery sent by a client

- **WHEN** a client sends a topic `Deliver` with an address of its choice
- **THEN** the message ends `done` and nothing is sent

#### Scenario: A delivery that is tried again

- **WHEN** the read of the store fails during a delivery, and works again a second later
- **THEN** the subscriber gets the message once, from a later attempt

#### Scenario: Subscribing twice

- **WHEN** an actor subscribes twice and a message is published
- **THEN** it gets the message once

#### Scenario: A value that is no message does not compile

- **WHEN** code builds a `Publish` with the typed helper from a value that is no message
- **THEN** the code does not compile

#### Scenario: The correlation id of a publish

- **WHEN** a message is published with the correlation id `c1`
- **THEN** each subscriber's message has the correlation id `c1`

### Requirement: A subscriber that is gone does not stop the others

A topic SHALL deliver to each subscriber in a turn of its own, so that a delivery that fails fails alone. A topic
SHALL watch itself and not its subscribers. It SHALL remove a subscriber whose delivery was rejected because the
subscriber does not exist. A delivery whose send is refused at the call SHALL be refused for good, send nothing,
and keep the subscriber. A delivery whose read of the store fails SHALL be tried again.

#### Scenario: A subscriber was deleted

- **WHEN** one of two subscribers is deleted and a message is published before the topic learned of it
- **THEN** the other subscriber gets the message, and the delivery to the deleted one ends `failed` alone

#### Scenario: A subscriber that went away is dropped

- **WHEN** a subscriber is deleted, a message is published, and the topic has handled the failure of that delivery
- **THEN** the topic's state no longer holds it, and a later publish makes no message for it

#### Scenario: The failures of a subscriber are not the topic's business

- **WHEN** a message of a subscriber fails
- **THEN** the topic gets no message about it

#### Scenario: A subscriber whose type is not registered

- **WHEN** an address whose type no node registered is subscribed and a message is published
- **THEN** the delivery to it ends `failed` with the cause `refused` and sends nothing, and the address stays
  subscribed

#### Scenario: A message the subscriber's type does not take

- **WHEN** a message of a type that a subscriber's type does not have is published
- **THEN** the delivery to that subscriber ends `failed` with the cause `refused`, and the subscriber stays
  subscribed

#### Scenario: A subscriber that never existed

- **WHEN** an address that names no actor is subscribed and a message is published
- **THEN** its delivery ends `failed` with the cause `commit`, the topic then drops it, and a second publish makes
  no message for it
