import React, { useEffect, useState } from 'react';
import { useSpring } from 'framer-motion';

interface AnimatedNumberProps {
  value: number;
  format?: (val: number) => string;
  className?: string;
}

export function AnimatedNumber({ 
  value, 
  format = (v) => Math.round(v).toLocaleString(), 
  className 
}: AnimatedNumberProps) {
  const spring = useSpring(value, { mass: 0.8, stiffness: 75, damping: 15 });
  const [displayValue, setDisplayValue] = useState(() => format(value));

  useEffect(() => {
    spring.set(value);
  }, [value, spring]);

  useEffect(() => {
    const unsubscribe = spring.on('change', (latest) => {
      setDisplayValue(format(latest));
    });
    return () => unsubscribe();
  }, [spring, format]);

  return <span className={className}>{displayValue}</span>;
}
