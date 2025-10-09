# local relay

- every tab can read and write to it. we just check that the event ids match. maybe have some attribution about which site put it in there.
- have a little settings page where you can send messages to your relay. maybe have a built-in ai-assisted query interface so you can "talk to your db"
- figure out what api we'd need to expose to developers. do they write their app just against this relay? is it different than a normal nostr app? so we also expose a bunch of window.nostr.??? this part feels likely like premature optimiation

# nostr event viewer

- can enter any nostr event with a kind specified in a NIP in the url bar / command ballette and it will render the event and give you some actinos to take on it if relevant. also blossom hash.

# nostr signer

- should we support users without an npub?
- would be really cool if we had a mode that would auto-login to every nostr site. there has to be some json signatures we could whitelist and auto-sign or whatever. would just be so cool if you go to a site for the first time and you're already logged in. i've never seen this. if you don't want this, open incognito

# relay stats

- pretty interactive dashboard showing average pings and disconnection rates and everything using local data
- maybe opt-in anonymized telemetry for things like this to collect some aggregate stats and share them publicly

# ad blocker

- this should be built directly into the browser
- i wonder if we could literally run ublock origin at least for starters

# sqlite

- i think we should implement the bun
- since we already have lmdb for nostrdb
  - perhaps we should also directly expose this.
  - perhaps we should re-implement indexeddb to be actually fast on top of lmdb?
- it would also be really nice to just get a filesystem. why not? or s3 api. we don't store files to database on the server, so why do we do it on the frontend? (maybe i just don't know the best practice here?)

# wallet

- i think we should support literally everything, but hide almost all of it behind advanced settings.
- show the balance in the ui by where chrome shows extensions?

# hotkeys

- would be neat to support a vim-mode natively from the start
- command pallette?

# marmot

- have some permissions e.g. the "add repo to vercel" flow ... you choose what group chats it can participate in

# moq

- build in a couple convenient APIs to make it trivial to build an E2E video or audio calling app, or live-streaming streaming app

# wasm

- how to get accessibility and rich text? is that browser responsibility of app responsibility?
- how to get support different ui framework? e.g. can we make like an adaptor for most popular golang ui framework or egui to be able to use use vello's rendering api?

# hypermedia

- could add hypermedia controls directly to our version of html? would be cool to do it at least for common nostr queries and actions.
- it would be awesome to make a new kind of app where you don't

# devtools

- network tab
- console tab
- db tab
- dom inspector

# chrome

- show loading state at the bottom like other browsers
- native(?) scrollbar
- command pallet
- good address bar
- settings page

# android

- can we save pwas to desktop? can we do this with our custom wasm component apps?

# progressive web apps

- how much can we improve these? what are their shortcomings on mobile?
  - https://chatgpt.com/c/68e53e21-9e48-8332-9247-48c3f732a505

# building in public

- i could start live-streaming my vibe coding sessions.
