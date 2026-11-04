export interface Coupon {
  code: string;
  percentOff: number;
  expiresAt: Date;
}

export function applyCoupon(total: number, coupon: Coupon, now: Date = new Date()): number {
  if (coupon.expiresAt < now) {
    return total;
  }
  return Math.round(total * (100 - coupon.percentOff)) / 100;
}
