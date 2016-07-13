pub struct Option(pub u8);

// pub const TTY_OP_END:Option = Option(0);
pub const VINTR:Option = Option(1);
pub const VQUIT:Option = Option(2);
pub const VERASE:Option = Option(3);
pub const VKILL:Option = Option(4);
pub const VEOF:Option = Option(5);
pub const VEOL:Option = Option(6);
pub const VEOL2:Option = Option(7);
pub const VSTART:Option = Option(8);
pub const VSTOP:Option = Option(9);
pub const VSUSP:Option = Option(10);
pub const VDSUSP:Option = Option(11);

pub const VREPRINT:Option = Option(12);
pub const VWERASE:Option = Option(13);
pub const VLNEXT:Option = Option(14);
pub const VFLUSH:Option = Option(15);
pub const VSWTCH:Option = Option(16);
pub const VSTATUS:Option = Option(17);
pub const VDISCARD:Option = Option(18);
pub const IGNPAR:Option = Option(30);
pub const PARMRK:Option = Option(31);
pub const INPCK:Option = Option(32);
pub const ISTRIP:Option = Option(33);
pub const INLCR:Option = Option(34);
pub const IGNCR:Option = Option(35);
pub const ICRNL:Option = Option(36);
pub const IUCLC:Option = Option(37);
pub const IXON:Option = Option(38);
pub const IXANY:Option = Option(39);
pub const IXOFF:Option = Option(40);
pub const IMAXBEL:Option = Option(41);
pub const ISIG:Option = Option(50);
pub const ICANON:Option = Option(51);
pub const XCASE:Option = Option(52);
pub const ECHO:Option = Option(53);
pub const ECHOE:Option = Option(54);
pub const ECHOK:Option = Option(55);
pub const ECHONL:Option = Option(56);
pub const NOFLSH:Option = Option(57);
pub const TOSTOP:Option = Option(58);
pub const IEXTEN:Option = Option(59);
pub const ECHOCTL:Option = Option(60);
pub const ECHOKE:Option = Option(61);
pub const PENDIN:Option = Option(62);
pub const OPOST:Option = Option(70);
pub const OLCUC:Option = Option(71);
pub const ONLCR:Option = Option(72);
pub const OCRNL:Option = Option(73);
pub const ONOCR:Option = Option(74);
pub const ONLRET:Option = Option(75);

pub const CS7:Option = Option(90);
pub const CS8:Option = Option(91);
pub const PARENB:Option = Option(92);
pub const PARODD:Option = Option(93);

pub const TTY_OP_ISPEED:Option = Option(128);
pub const TTY_OP_OSPEED:Option = Option(129);