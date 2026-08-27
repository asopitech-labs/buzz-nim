# Nimino formal models

Formal models make selected implementation contracts executable; they do not
replace the versioned contracts, implementation, tests, or runbooks.

- `inventory/` registers every active model and its retirement policy.
- `tla/` contains temporal state-machine models.
- `scenarios/` contains bounded TLC configurations.
- `evidence/` records checked commands and results; large checker output stays
  untracked.

Run a model with the command recorded in its inventory file.
