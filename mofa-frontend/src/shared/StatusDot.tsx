import React from 'react';
import { cn } from '../lib/utils';
import { motion } from 'framer-motion';

export type StatusType = 'Hot' | 'Healthy' | 'Closed' | 'Warming' | 'HalfOpen' | 'Busy' | 'Cold' | 'Unloaded' | 'Failed' | 'Open' | 'Unhealthy' | 'Connecting';

interface StatusDotProps {
  status: StatusType;
  className?: string;
}

export function StatusDot({ status, className }: StatusDotProps) {
  let colorClass = 'bg-text-dim';
  let animatePulse = false;

  switch (status) {
    case 'Hot':
    case 'Healthy':
    case 'Closed':
      colorClass = 'bg-accent-green shadow-[0_0_8px_rgba(16,185,129,0.6)]';
      break;
    case 'Warming':
    case 'HalfOpen':
    case 'Connecting':
      colorClass = 'bg-accent-yellow shadow-[0_0_8px_rgba(245,158,11,0.6)]';
      animatePulse = true;
      break;
    case 'Busy':
      colorClass = 'bg-accent-blue shadow-[0_0_8px_rgba(59,130,246,0.6)]';
      animatePulse = true;
      break;
    case 'Cold':
    case 'Unloaded':
      break;
    case 'Failed':
    case 'Open':
    case 'Unhealthy':
      colorClass = 'bg-accent-red shadow-[0_0_8px_rgba(239,68,68,0.6)]';
      break;
    default:
      break;
  }

  return (
    <motion.div
      className={cn('w-2.5 h-2.5 rounded-full', colorClass, className)}
      animate={animatePulse ? { opacity: [1, 0.5, 1] } : { opacity: 1 }}
      transition={animatePulse ? { repeat: Infinity, duration: 1.5, ease: 'easeInOut' } : {}}
    />
  );
}
