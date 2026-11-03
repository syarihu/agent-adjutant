import { total, CartItem } from "../cart/cart";
import { fetchStock } from "../catalog/stock";

export async function checkout(items: CartItem[]): Promise<number> {
  for (const item of items) {
    const stock = await fetchStock(item.sku);
    if (stock < item.quantity) {
      throw new Error(`out of stock: ${item.sku}`);
    }
  }
  return total(items);
}
