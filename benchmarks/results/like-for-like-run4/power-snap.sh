#!/usr/bin/env bash
# usage: power-snap.sh OUTFILE LABEL
{
echo "== $2 $(date '+%F %T')"
echo "date: $(date)"
echo "uptime: $(uptime)"
echo "tuned-adm: $(tuned-adm active)"
echo "governors:"; cat /sys/devices/system/cpu/cpufreq/policy*/scaling_governor | sort | uniq -c
echo "epp policy0: $(cat /sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference)"
echo "AC online: $(cat /sys/class/power_supply/AC*/online | tr '\n' ' ')"
} > "$1" 2>&1
