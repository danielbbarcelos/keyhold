import {
  AppWindow,
  Banknote,
  Bike,
  Bitcoin,
  BookOpen,
  Bot,
  Briefcase,
  Building2,
  Car,
  ChartLine,
  Cloud,
  Code,
  Cpu,
  CreditCard,
  Database,
  FileText,
  Film,
  Fingerprint,
  Gamepad2,
  Gift,
  Globe,
  GraduationCap,
  HardDrive,
  Headphones,
  Heart,
  Home,
  IdCard,
  KeyRound,
  Landmark,
  Laptop,
  Lock,
  Mail,
  MapPin,
  MessageCircle,
  Monitor,
  Music,
  Network,
  Package,
  Phone,
  Plane,
  Printer,
  Receipt,
  Router,
  ScrollText,
  Server,
  Shield,
  ShoppingCart,
  Smartphone,
  Smile,
  Star,
  Stethoscope,
  Store,
  Terminal,
  Ticket,
  Train,
  Tv,
  User,
  Users,
  Utensils,
  Wallet,
  Wifi,
  Wrench,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import type { RecordIcon } from '../api'
import { cn } from '@/lib/utils'

/**
 * The icons a record can wear, with the words people would search them by.
 *
 * A curated set rather than every lucide icon: the whole library would put a thousand icons in the
 * bundle of a program whose argument is that it carries very little, and a picker with a thousand
 * choices is a picker nobody finishes using.
 */
export const ICONS: Array<{ name: string; icon: LucideIcon; words: string }> = [
  { name: 'key-round', icon: KeyRound, words: 'chave senha login acesso' },
  { name: 'globe', icon: Globe, words: 'site web internet' },
  { name: 'mail', icon: Mail, words: 'email e-mail correio' },
  { name: 'code', icon: Code, words: 'código github git dev' },
  { name: 'terminal', icon: Terminal, words: 'ssh terminal shell' },
  { name: 'server', icon: Server, words: 'servidor host' },
  { name: 'database', icon: Database, words: 'banco dados postgres mysql' },
  { name: 'cloud', icon: Cloud, words: 'nuvem cloud aws' },
  { name: 'network', icon: Network, words: 'rede vpn' },
  { name: 'wifi', icon: Wifi, words: 'wifi wi-fi rede' },
  { name: 'router', icon: Router, words: 'roteador modem' },
  { name: 'hard-drive', icon: HardDrive, words: 'disco backup nas' },
  { name: 'cpu', icon: Cpu, words: 'processador máquina' },
  { name: 'laptop', icon: Laptop, words: 'notebook computador' },
  { name: 'monitor', icon: Monitor, words: 'computador desktop' },
  { name: 'smartphone', icon: Smartphone, words: 'celular telefone app' },
  { name: 'app-window', icon: AppWindow, words: 'aplicativo app sistema' },
  { name: 'bot', icon: Bot, words: 'robô agente api' },
  { name: 'shield', icon: Shield, words: 'segurança proteção' },
  { name: 'lock', icon: Lock, words: 'cadeado trancado' },
  { name: 'fingerprint', icon: Fingerprint, words: 'digital biometria' },
  { name: 'credit-card', icon: CreditCard, words: 'cartão crédito débito' },
  { name: 'wallet', icon: Wallet, words: 'carteira dinheiro' },
  { name: 'landmark', icon: Landmark, words: 'banco governo' },
  { name: 'banknote', icon: Banknote, words: 'dinheiro pagamento' },
  { name: 'bitcoin', icon: Bitcoin, words: 'cripto bitcoin' },
  { name: 'receipt', icon: Receipt, words: 'conta fatura recibo' },
  { name: 'chart-line', icon: ChartLine, words: 'investimento corretora' },
  { name: 'shopping-cart', icon: ShoppingCart, words: 'compras loja' },
  { name: 'store', icon: Store, words: 'loja comércio' },
  { name: 'package', icon: Package, words: 'entrega pacote' },
  { name: 'gift', icon: Gift, words: 'presente cupom' },
  { name: 'ticket', icon: Ticket, words: 'ingresso bilhete' },
  { name: 'briefcase', icon: Briefcase, words: 'trabalho empresa' },
  { name: 'building-2', icon: Building2, words: 'empresa prédio escritório' },
  { name: 'home', icon: Home, words: 'casa residência' },
  { name: 'user', icon: User, words: 'pessoa conta usuário' },
  { name: 'users', icon: Users, words: 'equipe grupo família' },
  { name: 'id-card', icon: IdCard, words: 'documento identidade' },
  { name: 'file-text', icon: FileText, words: 'nota documento texto' },
  { name: 'scroll-text', icon: ScrollText, words: 'certificado contrato' },
  { name: 'book-open', icon: BookOpen, words: 'livro leitura' },
  { name: 'graduation-cap', icon: GraduationCap, words: 'escola curso faculdade' },
  { name: 'stethoscope', icon: Stethoscope, words: 'saúde médico plano' },
  { name: 'heart', icon: Heart, words: 'saúde favorito' },
  { name: 'message-circle', icon: MessageCircle, words: 'chat mensagem conversa' },
  { name: 'phone', icon: Phone, words: 'telefone ligação' },
  { name: 'headphones', icon: Headphones, words: 'suporte áudio' },
  { name: 'music', icon: Music, words: 'música streaming' },
  { name: 'film', icon: Film, words: 'filme streaming vídeo' },
  { name: 'tv', icon: Tv, words: 'tv televisão streaming' },
  { name: 'gamepad-2', icon: Gamepad2, words: 'jogo game' },
  { name: 'plane', icon: Plane, words: 'viagem voo avião' },
  { name: 'train', icon: Train, words: 'trem transporte' },
  { name: 'car', icon: Car, words: 'carro veículo' },
  { name: 'bike', icon: Bike, words: 'bicicleta' },
  { name: 'map-pin', icon: MapPin, words: 'lugar endereço' },
  { name: 'utensils', icon: Utensils, words: 'comida restaurante delivery' },
  { name: 'printer', icon: Printer, words: 'impressora' },
  { name: 'wrench', icon: Wrench, words: 'ferramenta manutenção' },
  { name: 'star', icon: Star, words: 'estrela favorito' },
  { name: 'smile', icon: Smile, words: 'pessoal outro' },
]

/** The colours a record's icon can take: the interface's own, desaturated, nothing else. */
export const ICON_COLOURS: Array<{ value: string; label: string; tile: string; dot: string }> = [
  { value: 'sky', label: 'Céu', tile: 'bg-sky-soft text-sky', dot: 'bg-sky' },
  { value: 'sand', label: 'Areia', tile: 'bg-sand-soft text-sand', dot: 'bg-sand' },
  { value: 'stone', label: 'Pedra', tile: 'bg-stone-soft text-stone', dot: 'bg-stone' },
  { value: 'lilac', label: 'Lilás', tile: 'bg-lilac-soft text-lilac', dot: 'bg-lilac' },
  { value: 'mint', label: 'Menta', tile: 'bg-mint-soft text-mint', dot: 'bg-mint' },
  { value: 'rose', label: 'Rosa', tile: 'bg-rose-soft text-rose', dot: 'bg-rose' },
  { value: 'amber', label: 'Âmbar', tile: 'bg-amber-soft text-amber', dot: 'bg-amber' },
]

/** What a record written before records lost their type looks like, until somebody picks. */
const LEGACY: Record<string, RecordIcon> = {
  login: { name: 'key-round', colour: 'sky' },
  note: { name: 'file-text', colour: 'sand' },
  ssh_key: { name: 'terminal', colour: 'stone' },
  certificate: { name: 'scroll-text', colour: 'lilac' },
  card: { name: 'credit-card', colour: 'mint' },
}

export function iconOf(icon: RecordIcon | null | undefined, kind = 'item'): RecordIcon {
  if (icon && (icon.image || icon.name)) {
    return icon
  }

  return LEGACY[kind] ?? { name: 'file-text', colour: 'stone' }
}

export function lucideOf(name: string): LucideIcon {
  return ICONS.find((one) => one.name === name)?.icon ?? FileText
}

export function tileOf(colour: string): string {
  return ICON_COLOURS.find((one) => one.value === colour)?.tile ?? ICON_COLOURS[2].tile
}

const SIZES = {
  xs: 'size-[22px] rounded-md [&_svg]:size-[13px]',
  sm: 'size-8 rounded-lg [&_svg]:size-4',
  md: 'size-9 rounded-[10px] [&_svg]:size-[18px]',
  lg: 'size-12 rounded-xl [&_svg]:size-[22px]',
}

/** A record's icon, as a tile. */
export function RecordTile({
  icon,
  kind,
  size = 'sm',
  className,
}: {
  icon: RecordIcon | null | undefined
  kind?: string
  size?: keyof typeof SIZES
  className?: string
}) {
  const shown = iconOf(icon, kind)

  if (shown.image) {
    return (
      <img
        src={shown.image}
        alt=""
        className={cn('shrink-0 object-cover', SIZES[size], className)}
      />
    )
  }

  const Icon = lucideOf(shown.name)

  return (
    <span className={cn('grid shrink-0 place-items-center', SIZES[size], tileOf(shown.colour), className)}>
      <Icon />
    </span>
  )
}
