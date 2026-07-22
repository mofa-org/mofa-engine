export const motionVariants: any = {
  enter: {
    initial: { opacity: 0 },
    animate: { opacity: 1, transition: { duration: 0.4, ease: [0.25, 0.1, 0.25, 1] } },
    exit: { opacity: 0, transition: { duration: 0.3, ease: 'easeIn' } }
  },
  fade: {
    initial: { opacity: 0 },
    animate: { opacity: 1, transition: { duration: 0.3, ease: [0.25, 0.1, 0.25, 1] } },
    exit: { opacity: 0, transition: { duration: 0.2, ease: 'easeIn' } }
  },
  staggerContainer: {
    animate: {
      transition: {
        staggerChildren: 0.04
      }
    }
  },
  staggerItem: {
    initial: { opacity: 0 },
    animate: { opacity: 1, transition: { duration: 0.3, ease: 'easeOut' } }
  },
  buttonTap: {
    scale: 0.98
  }
};
