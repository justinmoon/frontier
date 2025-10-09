# stragegy

1. get a solid browser that can run the "nostr web"
2. add native nostr support to the browser
3. traditional webdev improvements e.g. sqlite
4. a/v
5. nns
6. wasm apps (can make parallel experiments on this)
7. android

# road map

- local nostr relay / nostrdb
- js engine
  - render ants
  - render nostrudel
  - render primal
  - render damus
- ui test harness
- wpt tests in ci
- signer
- can enter any nostr event with a kind specified in a NIP in the url bar / command ballette and it will render the event and give you some actinos to take on it if relevant. also blossom hash.

# tech debt

- develop nns separately until it works
- move `DOM_BOOTSTRAP` to it's own file
- get rid of `unsafe` invocations
- our integration tests don't spawn real relays etc. they should.
- when creating nns claims in tests it seems like we're constructing them almost by hand. could use better helper functions
- figure out how to use rust-nostr with our nns-tls scheme
- i should study more nostr react apps and get familiar with how they load data. then try to make that into built-in html or css things.

# notes

- Mosaic spec uses Noise IK with ed25519 keys; we can swap to secp256k1 using Noise’s flexibility (Noise supports custom curves)
  - i had this note saved ... i think it's talking about quic?
