import React from 'react';
import { cn } from '../lib/utils';
import { motion, HTMLMotionProps } from 'framer-motion';

interface CardProps extends HTMLMotionProps<'div'> {
  interactive?: boolean;
}

export const Card = React.forwardRef<HTMLDivElement, CardProps>(
  ({ className, interactive, ...props }, ref) => {
    return (
      <motion.div
        ref={ref}
        className={cn(
          'bg-white border border-black/5 rounded-[var(--radius-card)] shadow-[0_2px_8px_rgba(0,0,0,0.04),0_0_1px_rgba(0,0,0,0.06)] overflow-hidden transition-all duration-150 ease-out',
          interactive && 'hover:border-black/10 hover:-translate-y-[2px] hover:shadow-[0_8px_24px_rgba(0,0,0,0.08)] cursor-pointer',
          className
        )}
        {...props}
      />
    );
  }
);
Card.displayName = 'Card';
