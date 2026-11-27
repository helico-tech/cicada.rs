## ADDED Requirements

### Requirement: The inspector inside the tray

With its feature `inspector`, the tray SHALL open the inspector in its own process on its connected context, else its
current one, SHALL bring an open inspector to the front without changing its context, and SHALL close the inspector,
with its pending refreshes, before it quits.

#### Scenario: Open twice

- **WHEN** the tray is online with `prod` and the user picks Open Inspector…, switches the inspector to `staging`, and
  picks Open Inspector… again
- **THEN** one inspector window shows `staging` and comes to the front

#### Scenario: Quit with the inspector open

- **WHEN** the inspector is open and refreshing, and the user quits the tray
- **THEN** the inspector closes, its refresh is dropped, and the tray quits as before

#### Scenario: Without the feature

- **WHEN** the tray is built without the feature `inspector`
- **THEN** the menu has no Open Inspector… item
