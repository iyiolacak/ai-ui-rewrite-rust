#!/usr/bin/env bash
# samples mean scaling_cur_freq (MHz) across cpus every 5 s
P=$HOME/.cache/ai-ui-rewrite-rust/results/perf-profile
while true; do
  f=$(cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_cur_freq | awk '{s+=$1/1000; n++} END {printf "%.0f", s/n}')
  echo "$(date +%T) $f" >> $P/freq.txt
  sleep 5
done
