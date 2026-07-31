import React from 'react';
import { cn } from '../lib/utils';
import { Capability } from '../engine/types';

interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: 'blue' | 'purple' | 'cyan' | 'yellow' | 'green' | 'red' | 'default';
  capability?: Capability;
}

export function Badge({ className, variant = 'default', capability, children, ...props }: BadgeProps) {
  let finalVariant = variant;
  
  if (capability) {
    const capLower = capability.toLowerCase();
    switch (capLower) {
      case 'chat': finalVariant = 'blue'; break;
      case 'tts': finalVariant = 'purple'; break;
      case 'asr': finalVariant = 'cyan'; break;
      case 'imagegen': case 'image_gen': finalVariant = 'yellow'; break;
      case 'embedding': finalVariant = 'green'; break;
      default: finalVariant = 'default';
    }
  }

  return (
    <span
      className={cn(
        'inline-flex items-center px-2 py-0.5 rounded text-xs font-medium border tracking-[0.01em]',
        finalVariant === 'blue' && 'bg-accent-blue/10 text-accent-blue border-accent-blue/20',
        finalVariant === 'purple' && 'bg-accent-purple/10 text-accent-purple border-accent-purple/20',
        finalVariant === 'cyan' && 'bg-accent-cyan/10 text-accent-cyan border-accent-cyan/20',
        finalVariant === 'yellow' && 'bg-accent-yellow/10 text-accent-yellow border-accent-yellow/20',
        finalVariant === 'green' && 'bg-accent-green/10 text-accent-green border-accent-green/20',
        finalVariant === 'red' && 'bg-accent-red/10 text-accent-red border-accent-red/20',
        finalVariant === 'default' && 'bg-background-secondary text-text-secondary border-border-strong',
        className
      )}
      {...props}
    >
      {capability || children}
    </span>
  );
}
