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
          'bg-background-card border border-border-subtle rounded-[var(--radius-card)] shadow-md overflow-hidden transition-all duration-150 ease-out',
          interactive && 'hover:border-border-strong hover:shadow-lg cursor-pointer',
          className
        )}
        {...props}
      />
    );
  }
);
Card.displayName = 'Card';
