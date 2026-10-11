# Findings
- pump pushes 1 row/frame; draw uploads only row_rgba(0) at head, throttled 33 ms -> rows pushed on throttled draws lost.
- generation bumps on push/rebuild/no-data-colour; rebuilds bumps on all but push. So with rebuilds unchanged, generation delta == rows pushed.
