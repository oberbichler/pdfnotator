#set page(paper: "a4", margin: (x: 2.2cm, y: 2.4cm), numbering: "1")
#set text(font: "New Computer Modern", size: 10.5pt)
#set par(justify: true, leading: 0.7em)
#set heading(numbering: "1.")
#show heading.where(level: 1): set block(above: 1.6em, below: 0.9em)

#align(center)[
  #text(18pt, weight: "bold")[Cloud Migration Proposal]\
  #v(2pt)
  #text(11pt, fill: luma(90))[Platform Team · November 2026]
]
#v(10pt)

= Summary
We propose moving the order platform from the on-premise data centre to a managed
Kubernetes service within two quarters. The migration reduces operating costs by
roughly 30 %, removes the hardware refresh planned for next year and gives every
team self-service environments for testing.

= Current situation
The platform runs on twelve virtual machines that are patched and scaled by hand.
Releases need a maintenance window, and load peaks in December regularly exceed
the reserved capacity. Monitoring is spread across three tools, which makes
incidents slow to diagnose.

= Approach
The migration happens in three phases. Each phase ends with a production release,
so the business benefits early and the risk of a big-bang cut-over is avoided.

#table(
  columns: (auto, 1fr, auto),
  inset: 7pt,
  stroke: 0.5pt + luma(180),
  fill: (_, y) => if y == 0 { luma(235) },
  [*Phase*], [*Scope*], [*Duration*],
  [1 · Foundation], [Landing zone, CI/CD pipelines, observability stack], [6 weeks],
  [2 · Services], [Containerise and move the twelve services], [10 weeks],
  [3 · Data], [Database migration and decommissioning], [8 weeks],
)

= Costs and benefits
Running costs drop from 41 k€ to 29 k€ per month once phase 3 is complete. The
one-off effort is estimated at 380 person-days, including training for the
operations team. Faster releases and on-demand scaling are not included in this
figure but are expected to be the larger benefit.

= Risks
The main risk is the database migration in phase 3. We mitigate it with a
replicated read model that runs in parallel for four weeks before the switch.
