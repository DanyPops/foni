# Joe Rogan Reads a Take-Home Assignment
### *A live commentary transcript*

---

*[settles into chair, picks up paper, adjusts mic]*

Alright. Let's see what we got here.

---

> **"TAKE-HOME ASSIGNMENT — Dynamic Booking & Reservation Engine. Return window: 4 days from receipt."**

Four days. Okay. That's actually... that's decent. They're not being animals about it. Four days.

---

> **"This is a deliberately open-ended problem. We care far more about how you reason about correctness, concurrency, and trade-offs than about feature completeness or polish. A smaller scope done correctly beats a large scope with race conditions."**

Okay hold on. *"A smaller scope done correctly beats a large scope with race conditions."* That is a genuinely interesting thing to say in a job spec. Most companies just want you to ship as much as possible and figure out the disasters later. These people are telling you upfront — we want you to *think*. That's rare. That's actually rare, man.

---

> **"We are launching a ticket marketplace — think Ticketmaster, or a B2B appointment-scheduling platform. Your task is to build a monolithic backend API from scratch that handles event creation, seat reservations, temporary holds, and dynamic pricing."**

Monolithic. From scratch. Okay. They're not asking you to wire up microservices and a message queue and a Kubernetes cluster and seventeen different AWS services. They want a *thing* that *works*. That's... I respect that. That's how you actually learn if someone can build software — you strip away all the infrastructure scaffolding and say: here, make it go.

---

> **"These three mechanisms are the heart of the assignment. They are intentionally the difficult part."**

*Intentionally the difficult part.* They're telling you to your face. I love that. No games. No hidden gotchas. They're going: here are the hard problems, go solve them.

---

> **"1. 5-Minute Cart Lock (State Lifecycle). When a customer selects a ticket, that ticket must be locked for exactly 5 minutes so they can complete a checkout form. If the purchase is not completed within 5 minutes, the system must automatically release the ticket back into the public pool."**

Okay so anyone who's ever tried to buy concert tickets knows this. You grab the ticket, the clock starts, and if you screw around for too long it goes back. The question is — how do you BUILD that? Because it sounds simple but think about it. You need something watching the clock. You need something that knows the lock is there. You need it to clean itself up without anyone asking it to. That's actually... there's a lot of ways to get that wrong.

---

> **"2. Dynamic Surge Pricing. Ticket price changes automatically with demand. For every 10% of total tickets sold for an event, the price of the remaining tickets increases by 5%."**

So Uber surge pricing but for concerts. For every ten percent of tickets sold, the price of the rest goes up five percent. Okay so if you sell half the event, the price has gone up... *[counts quietly]* ...twenty-five percent. Yeah. The math is clean. The implementation is not.

---

> **"Price increases must apply instantly to incoming requests… …but must not affect users who already hold a locked ticket. A user is quoted and charged the price that was active when their lock was created."**

Oh. *Oh.* So you have two realities running at the same time. There's the live price — which is moving — and then there's the locked price — which is frozen at the moment the user grabbed the ticket. Those two things have to coexist. And you can never confuse them. If you charge someone a higher price than what they were shown when they locked in, that's a breach of trust, that's potentially a legal issue, and you've also just built a bug that will destroy your company's reputation in about forty-eight hours. So... yeah. That boundary has to be airtight.

---

> **"3. High-Concurrency Checkout. Scenario: only 5 tickets remain. 100 users hit the checkout endpoint at the same millisecond. The system must guarantee exactly 5 succeed and 95 receive a clean 'Sold Out' response. No double-bookings. No negative inventory. No crashes. This must hold under genuine concurrent load."**

Oh man. You gotta be shitting me, man. I've been vibe coding for the past six months — who remembers how to write code?!

*[laughs]*

No, seriously though. A hundred users. Same millisecond. Five tickets. This is the kind of problem that — if you haven't thought hard about concurrency before — is going to completely break your brain. Because your first instinct is: I'll just check if there are tickets available, and if yes, I'll sell one. But that check-then-act — that's a race condition. Between the check and the act, ninety-nine other people did the same check and got the same answer. Now you've sold the same five tickets a hundred times. You have negative inventory. You've got a hundred angry people and a company that's in legal trouble. The fix is atomic operations. You have to make the "check AND sell" a single indivisible thing. And that is genuinely hard to reason about correctly.

---

> **"Frontend: UI / UX. Alongside the backend, build a small client that consumes your API and makes the booking flow tangible. We are not judging visual flourish — we are judging whether the interface honestly reflects the system's state."**

*Wait.* You also gotta build a UI? *[laughs]* Man. It's a full stack assignment. Okay. Okay but look — they said it right there: we're not judging visual flourish. They want to see if the UI *tells the truth*. Does the countdown actually count down? Does the price actually change when it should? Does it show "Sold Out" when it's sold out, not when it *thinks* it's sold out based on stale data? That's a design problem as much as a coding problem.

---

> **"Event view. Show current availability and the live price. The price on screen must reflect surge changes — if you hold the page open while inventory sells down elsewhere, the displayed price should update (poll or refresh is fine; state your approach)."**

*"Poll or refresh is fine; state your approach."* That's a beautiful sentence. They're not demanding WebSockets. They're not requiring real-time streaming. They're saying: pick a strategy, justify it, own it. That's how real engineering works. There's no always-right answer. There's only: here's my reasoning, here are the trade-offs, here's what I chose and why.

---

> **"Reserve / cart flow. When a user reserves a ticket, show a visible countdown timer for the 5-minute lock and the price they locked in. This locked price must not change even if the live event price rises while they sit in checkout."**

So the UI has to know about the two-reality problem too. The screen is showing you the locked price, the event in the background might be at a completely different price now, and the interface has to stay honest about both without confusing the user. That's subtle work.

---

> **"Lock expiry. When the countdown hits zero, the UI must clearly communicate that the hold was released and return the user to the event view — no silent failures, no checking out against an expired lock."**

*No silent failures.* That should be the motto of every software team in existence, honestly. The worst bugs aren't crashes — crashes you can see. The worst bugs are silent. The ones where the system just quietly does the wrong thing and nobody notices until it's catastrophically too late.

---

> **"Sold-out state. If checkout fails because inventory is gone, surface a clean, unambiguous 'Sold Out' message rather than a raw error."**

Don't show people a stack trace. Don't show them a JSON error blob. Tell them what happened in English. This is — I feel like this is such a basic thing but you would not believe how many apps fail at it. You've seen it. You try to buy something and you get "Error 500: Internal Server Error." What does that mean? Nothing. It means nothing to a normal human being.

---

> **"Mobile Responsiveness. The UI must be fully responsive and usable on a mobile viewport (~375px wide) as well as desktop. Assume a meaningful share of buyers are on phones. No horizontal scrolling, no overlapping elements, no content cut off at narrow widths."**

Yeah. Because people are buying concert tickets on their phones at eleven PM with one hand while they're in bed. That's the reality. You design for desktop only, you've cut off half your users before they even start.

---

> **"What We Evaluate. Correctness under concurrency. The inventory invariant must never break. This is the single most important thing."**

The inventory invariant must never break. This is the single most important thing. They said it. They put it first. They bolded it, essentially. That's not an accident. That's them telling you: if you get everything else right and you get this wrong, you failed. No partial credit on negative inventory.

---

> **"Quality of the locking / expiry design. How you reason about time, atomicity, and cleanup without external tools."**

*Without external tools.* No Redis for the lock. No Celery for the expiry job. No Kafka. Just your brain, the language, and whatever's in the standard library. That's the test. Can you reason about this problem from first principles?

---

> **"Testing. Particularly a test that demonstrates the 100-users-5-tickets scenario behaving correctly."**

They want to SEE it. Not "trust me it works." Show me. Write a test that spawns a hundred goroutines — or threads, or whatever — all hitting the same endpoint at the same time, and prove that exactly five of them win. That test is the whole assignment in one file, really.

---

> **"Communication. A clear README explaining your design decisions and trade-offs."**

This one. This is underrated. I've seen brilliant engineers who cannot explain what they built. And I've seen mediocre engineers who can explain their thinking so clearly that you trust them immediately. The README is where you show your reasoning. Don't skip it. Don't write three sentences. Walk them through how you thought about the problem.

---

> **"What We Are NOT Looking For. Authentication, user accounts, payment integration. Pixel-perfect or heavily designed visuals. Exhaustive feature coverage."**

They're giving you permission to stay focused. That's a gift. Take it.

---

> **"Good luck. We are excited to see how you think."**

*"We are excited to see how you think."* Not what you build. Not how fast you build it. How you *think*. That's everything, man. That's the whole game.

---

---

## Rogan's Guide — "Yuri, listen"

*[leans forward, elbows on the table]*

Okay Yuri, listen. Before you open your IDE, before you think about frameworks, before you even think about what language you're using — I want you to step back and treat this as a pure logical problem first.

You are booking a seat at an event. That's it. Somebody wants a seat. There's a limited number of seats. When one person gets it, nobody else can have it. That's the domain. Understand the domain before you touch a single line of code.

Don't rush into HTTP servers. Don't rush into SQL databases. Don't rush into anything. Sit down, grab a piece of paper, and ask yourself: what are the *things* in this system? What can they do? What rules can never be broken? You've got an Event. You've got a Seat — or a Ticket, same thing. You've got a Reservation — which is either locked, purchased, or expired. Draw the states. Draw the transitions between them. When does a locked reservation become purchased? When does it expire? What happens to inventory at each transition? Write that down before you write a function.

The concurrency problem? That's not a coding problem first. That's a thinking problem. What does it mean for two operations to conflict? When do they conflict? What's the minimal constraint you need to prevent the conflict? Answer that in plain English before you answer it in code.

The surge pricing? Same thing. What's the rule? For every ten percent of inventory sold, price goes up five percent. But the lock creates a price snapshot. So you have: current price, and locked price. They evolve independently after the lock is created. Can you draw a timeline of how those two values change? Do it. Then code it.

Now — about the AI thing. You've got a full spec. You've got user stories. Every edge case is written down for you. The task isn't "ask an AI agent to generate the code." That's the trap. The task is *decision making, architecture, testing*. Where do you put the lock? How do you make the inventory check atomic? What test proves the concurrency works? Those are judgment calls. Those are engineering decisions. An AI can generate code all day long — it can't make those calls *for your specific situation* without you understanding the trade-offs first.

The people who read your submission are going to look at your README and they're going to know immediately whether you understood the problem or just generated something that compiles. Write the README like you're explaining the system to a smart friend who's never seen it. Tell them what you were scared of. Tell them where you had to make a trade-off and what you traded. That's the signal they're looking for.

Four days, man. That's enough time. Start with the domain model. Work outward. Don't skip the test.

*Good luck.*

---

*[takes a sip of something]*

Pull up a chair. Have some DMT. Let's talk about inventory invariants.

*[laughs]*
