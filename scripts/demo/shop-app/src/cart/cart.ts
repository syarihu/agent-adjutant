import { applyCoupon, Coupon } from "./coupon";

export interface CartItem {
  sku: string;
  price: number;
  quantity: number;
}

export function subtotal(items: CartItem[]): number {
  return items.reduce((sum, item) => sum + item.price * item.quantity, 0);
}

export function total(items: CartItem[], coupon?: Coupon): number {
  const sum = subtotal(items);
  return coupon ? applyCoupon(sum, coupon) : sum;
}
