# Take-Home Assignment: Dynamic Booking & Reservation Engine

**Return window:** 4 days from receipt.

This is a deliberately open-ended problem. We care far more about how you reason about correctness, concurrency, and trade-offs than about feature completeness or polish. A smaller scope done correctly beats a large scope with race conditions.

---

## Overview

We are launching a ticket marketplace (think Ticketmaster, or a B2B appointment-scheduling platform). Your task is to build a monolithic backend API from scratch that handles event creation, seat reservations, temporary holds, and dynamic pricing.

---

## Core Requirements

These three mechanisms are the heart of the assignment. They are intentionally the difficult part.

### 1. 5-Minute Cart Lock (State Lifecycle)

- When a customer selects a ticket, that ticket must be locked for exactly 5 minutes so they can complete a checkout form.
- If the purchase is not completed within 5 minutes, the system must automatically release the ticket back into the public pool.

### 2. Dynamic Surge Pricing

- Ticket price changes automatically with demand. For every 10% of total tickets sold for an event, the price of the remaining tickets increases by 5%.
- Price increases must apply instantly to incoming requests…
- …but must not affect users who already hold a locked ticket. A user is quoted and charged the price that was active when their lock was created.

### 3. High-Concurrency Checkout

- Scenario: only 5 tickets remain. 100 users hit the checkout endpoint at the same millisecond.
- The system must guarantee exactly 5 succeed and 95 receive a clean "Sold Out" response.
- No double-bookings. No negative inventory. No crashes. This must hold under genuine concurrent load.

---

## Frontend: UI / UX

Alongside the backend, build a small client that consumes your API and makes the booking flow tangible. We are not judging visual flourish — we are judging whether the interface honestly reflects the system's state and handles the hard moments (locks expiring, prices moving, tickets selling out) gracefully.

Tech is your choice. Keep it lightweight.

### Required Screens & Behavior

- **Event view.** Show current availability and the live price. The price on screen must reflect surge changes — if you hold the page open while inventory sells down elsewhere, the displayed price should update (poll or refresh is fine; state your approach).
- **Reserve / cart flow.** When a user reserves a ticket, show a visible countdown timer for the 5-minute lock and the price they locked in. This locked price must not change even if the live event price rises while they sit in checkout.
- **Lock expiry.** When the countdown hits zero, the UI must clearly communicate that the hold was released and return the user to the event view — no silent failures, no checking out against an expired lock.
- **Sold-out state.** If checkout fails because inventory is gone, surface a clean, unambiguous "Sold Out" message rather than a raw error.

### Mobile Responsiveness

- The UI must be fully responsive and usable on a mobile viewport (~375px wide) as well as desktop. Assume a meaningful share of buyers are on phones.
- Touch targets (reserve / checkout buttons) should be comfortably tappable; the countdown and price must remain legible on small screens.
- No horizontal scrolling, no overlapping elements, no content cut off at narrow widths. Layout should reflow, not just shrink.
- We will resize the browser and/or open it on a phone — design for both, mobile-first if that's your instinct.

### What We Care About (UI)

- Honest representation of server state — the client never claims a ticket is held or sold when the backend disagrees.
- Graceful handling of the edge moments: expiry, surge mid-session, and sold-out races.
- Clear, calm feedback. Loading, success, and failure states are all accounted for.

---

## Suggested API Surface

Endpoint names and shapes are up to you — this is a guide, not a spec.

1. `POST /events` — create an event (capacity, base price).
2. `POST /events/:id/reserve` — lock a ticket; returns a hold ID and the locked-in price.
3. `POST /reservations/:id/checkout` — finalize a purchase against a held lock.
4. `GET /events/:id` — current availability and live price.

---

## What We Evaluate

In rough order of importance:

- **Correctness under concurrency.** The inventory invariant must never break. This is the single most important thing.
- **Quality of the locking / expiry design.** How you reason about time, atomicity, and cleanup without external tools.
- **Clarity of the surge-pricing logic,** especially the boundary case of a price changing while a lock is held.
- **UI / UX that honestly reflects system state,** including the lock countdown, live price, and a responsive layout that works on mobile.
- **Code quality and structure.** Readable, organized, sensibly separated concerns.
- **Testing.** Particularly a test that demonstrates the 100-users-5-tickets scenario behaving correctly.
- **Communication.** A clear README explaining your design decisions and trade-offs.

---

## What We Are NOT Looking For

- Authentication, user accounts, payment integration.
- Pixel-perfect or heavily designed visuals — a clean, functional, responsive UI is enough.
- Exhaustive feature coverage. Depth on the three core mechanisms beats breadth.

---

## Deliverables

1. A Git repository (GitHub link or zip) with your source code.
2. A README covering: how to run it (backend and frontend), your approach to each of the three core requirements, and the trade-offs you made.
3. At least one test that exercises the high-concurrency checkout scenario.

---

## Submission

Reply to this email with your repository link or attachment. If anything is ambiguous, state your assumptions in the README and proceed — we treat reasonable assumptions as a positive signal, not a problem.

Good luck. We are excited to see how you think.
