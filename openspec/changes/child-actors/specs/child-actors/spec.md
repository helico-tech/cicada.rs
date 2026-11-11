## ADDED Requirements

### Requirement: A turn creates a child or an independent actor

`turn.create_child` SHALL create an actor whose parent is the actor that runs the turn. `turn.create` and every
create from outside a turn SHALL create an actor without a parent. The parent SHALL be stored with the actor and SHALL
never change. Like every create, a child create is applied before the turn's sends and is atomic with the commit.

#### Scenario: A child is created

- **WHEN** a turn of `batches/b1` calls `create_child` for `batch-items/b1:0` and commits
- **THEN** `batch-items/b1:0` exists and its parent is `batches/b1`

#### Scenario: An independent actor is created

- **WHEN** a turn calls `create`, or a client sends `PUT /actors/{type}/{key}`
- **THEN** the new actor has no parent

#### Scenario: The turn fails

- **WHEN** a turn calls `create_child` and its handler then fails
- **THEN** no child is created

### Requirement: A child create never takes over an existing actor

A child create of an actor that already exists as a child of the same parent SHALL leave it untouched. A child create
of an actor that exists with another parent, or with none, SHALL reject the commit: nothing is applied and the turn's
message ends `failed` with a reason that names the actor. Creates of one address in one turn that disagree on being a
child SHALL be refused: the second call returns `TurnError::Invalid`, and a store given such a commit answers
`StoreError::Invalid`.

#### Scenario: The child exists already

- **WHEN** a parent calls `create_child` for a child it created before
- **THEN** the commit succeeds and the child keeps its state and its mailbox

#### Scenario: The actor belongs to nobody or to someone else

- **WHEN** a turn calls `create_child` for an actor that was created as independent, or as a child of another actor
- **THEN** the turn's message ends `failed`, none of the turn's effects are applied, and the actor is unchanged

#### Scenario: One turn creates the same address both ways

- **WHEN** a handler calls `create` and then `create_child` for the same address
- **THEN** the second call returns an error and buffers nothing

### Requirement: Removing an actor removes its descendants

Deleting an actor (`Store::delete`, `ActorSystem::delete_actor`, `DELETE /actors/{type}/{key}`) or stopping it
(`turn.stop()`) SHALL remove the actor and every descendant, with their mailboxes, in one transaction. Actors outside
the tree SHALL not change. Pause, resume and purge SHALL not reach descendants.

#### Scenario: An operator deletes a parent

- **WHEN** `a` has the child `b`, `b` has the child `c`, and a client deletes `a`
- **THEN** `a`, `b` and `c` are gone and no other actor changed

#### Scenario: A parent stops

- **WHEN** a turn of a parent creates a child and calls `stop()`
- **THEN** after the commit the parent and all its children are gone, the new child included

#### Scenario: A child stops

- **WHEN** a child calls `stop()`
- **THEN** the child and its own descendants are gone; its parent and its siblings stay

#### Scenario: A removal races with commits

- **WHEN** an actor is deleted while turns of its descendants commit creates of further children
- **THEN** afterwards no actor exists whose parent is gone

### Requirement: A removed descendant ends like a deleted actor

Every pending message of a removed descendant SHALL end `purged`, including one being handled. A turn that runs on a
removed descendant SHALL get `LeaseLost` at commit and apply nothing. The number of purged messages that a delete
reports SHALL cover the whole tree.

#### Scenario: Pending messages in the tree

- **WHEN** a parent and its child each have a pending message and the parent is deleted
- **THEN** both messages have the outcome `purged` and the delete reports two purged messages

#### Scenario: A message sent by the stopping turn

- **WHEN** a turn of a parent sends a message to its child and calls `stop()`
- **THEN** that message has the outcome `purged`

#### Scenario: A turn runs on a descendant

- **WHEN** a turn is running on a child while its parent is deleted
- **THEN** the child's commit answers `LeaseLost` and its message has the outcome `purged`

### Requirement: The view of an actor names its parent

The view of an actor (`GET /actors/{type}/{key}`, `ActorSystem::get_actor`) SHALL carry `parent`: the parent's address,
or null for an independent actor.

#### Scenario: A child

- **WHEN** a client gets a child
- **THEN** the body has `parent` with the address of the parent

#### Scenario: An independent actor

- **WHEN** a client gets an actor that was created with `create` or over HTTP
- **THEN** the body has `parent` with the value null

### Requirement: The children of an actor can be listed

`GET /actors/{type}/{key}/children?cursor=&limit=` SHALL list the addresses of the actor's direct children in address
order, paged like the other listings, and answer `404` with problem type `no-such-instance` for an actor that does not
exist. It is a read route. `Store::children`, `ActorSystem::list_children` and `ActorClient::children` SHALL give the
same listing.

#### Scenario: Direct children only

- **WHEN** `a` has the children `b1` and `b2`, and `b1` has the child `c`
- **THEN** the children of `a` are `b1` and `b2`, in that order, and `c` is listed only under `b1`

#### Scenario: Paging

- **WHEN** an actor has more children than `limit`
- **THEN** the page has `limit` addresses and a `next` cursor that continues after the last one

#### Scenario: An unknown actor

- **WHEN** a client gets `/actors/{type}/{key}/children` of an actor that does not exist
- **THEN** the answer is `404` with problem type `no-such-instance`

### Requirement: The command line lists children

`cicada get children ADDR` SHALL print the direct children of the actor, with the paging flags of the other listings,
and fail with exit code 1 when the actor does not exist. The wide table and the JSON and YAML output of an actor SHALL
show its parent.

#### Scenario: Children as a table

- **WHEN** an operator runs `cicada get children batches/b1`
- **THEN** the output is one row per child address

### Requirement: The stores migrate to the parent column

Both SQL stores SHALL add the parent column and its index in an appended migration. Actors that existed before the
migration SHALL have no parent.

#### Scenario: A database of the previous version is opened

- **WHEN** a store opens a database of the previous schema version that holds actors
- **THEN** the database is at the new version, the actors are still there and each has no parent
