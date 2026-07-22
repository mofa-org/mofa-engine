import React from 'react';
import { cn } from '../lib/utils';
import { motion, HTMLMotionProps } from 'framer-motion';
import { motionVariants } from '../lib/motion';

interface ButtonProps extends HTMLMotionProps<'button'> {
  variant?: 'primary' | 'secondary' | 'ghost';
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = 'primary', ...props }, ref) => {
    return (
      <motion.button
        ref={ref}
        whileTap={props.disabled ? undefined : motionVariants.buttonTap}
        className={cn(
          'inline-flex items-center justify-center rounded-[var(--radius-small)] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-blue disabled:opacity-50 disabled:pointer-events-none',
          'h-10 px-4 py-2 text-sm shadow-sm',
          variant === 'primary' && 'bg-black text-white hover:bg-gray-800 shadow-md',
          variant === 'secondary' && 'bg-white text-text-primary hover:bg-gray-50 border border-black/10',
          variant === 'ghost' && 'hover:bg-black/5 text-text-secondary hover:text-text-primary shadow-none',
          className
        )}
        {...props}
      >
        <span className="relative z-10 flex items-center justify-center gap-2 w-full">{props.children as React.ReactNode}</span>
      </motion.button>
    );
  }
);
Button.displayName = 'Button';
