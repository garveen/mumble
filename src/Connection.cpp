// Copyright The Mumble Developers. All rights reserved.
// Use of this source code is governed by a BSD-style license
// that can be found in the LICENSE file at the root of the
// Mumble source tree or at <https://www.mumble.info/LICENSE>.

#include "Connection.h"
#include "Mumble.pb.h"
#include "SSL.h"

#include <QtCore/QtEndian>
#include <QtNetwork/QHostAddress>

#ifdef USE_WEBSOCKET
#	include <QtWebSockets/QWebSocket>
#endif

#ifdef Q_OS_WIN
#	include <qos2.h>
#else
#	include <netinet/in.h>
#	include <netinet/tcp.h>
#	include <sys/socket.h>
#	include <sys/types.h>
#endif

#ifdef Q_OS_WIN
HANDLE Connection::hQoS = nullptr;
#endif

Connection::Connection(QObject *p, QSslSocket *qtsSock) : QObject(p) {
	qtsSocket = qtsSock;
#ifdef USE_WEBSOCKET
	qwsSocket = nullptr;
#endif
	qtsSocket->setParent(this);
	iPacketLength        = -1;
	bDisconnectedEmitted = false;
	csCrypt              = std::make_unique< CryptStateOCB2 >();

	static bool bDeclared = false;
	if (!bDeclared) {
		bDeclared = true;
		qRegisterMetaType< QAbstractSocket::SocketError >("QAbstractSocket::SocketError");
	}

	int nodelay = 1;
	setsockopt(static_cast< int >(qtsSocket->socketDescriptor()), IPPROTO_TCP, TCP_NODELAY,
			   reinterpret_cast< char * >(&nodelay), static_cast< socklen_t >(sizeof(nodelay)));

	connect(qtsSocket, SIGNAL(errorOccurred(QAbstractSocket::SocketError)), this,
			SLOT(socketError(QAbstractSocket::SocketError)));
	connect(qtsSocket, SIGNAL(encrypted()), this, SIGNAL(encrypted()));
	connect(qtsSocket, SIGNAL(readyRead()), this, SLOT(socketRead()));
	connect(qtsSocket, SIGNAL(disconnected()), this, SLOT(socketDisconnected()));
	connect(qtsSocket, SIGNAL(sslErrors(const QList< QSslError > &)), this,
			SLOT(socketSslErrors(const QList< QSslError > &)));
	qtLastPacket.restart();
#ifdef Q_OS_WIN
	dwFlow = 0;
#endif
}

#ifdef USE_WEBSOCKET
Connection::Connection(QObject *p, QWebSocket *qwsSock) : QObject(p) {
	qtsSocket = nullptr;
	qwsSocket = qwsSock;
	qwsSocket->setParent(this);
	iPacketLength        = -1;
	bDisconnectedEmitted = false;
	csCrypt              = std::make_unique< CryptStateOCB2 >();

	static bool bDeclared = false;
	if (!bDeclared) {
		bDeclared = true;
		qRegisterMetaType< QAbstractSocket::SocketError >("QAbstractSocket::SocketError");
	}

	connect(qwsSocket, &QWebSocket::binaryMessageReceived, this, &Connection::wsBinaryMessageReceived);
	connect(qwsSocket, &QWebSocket::errorOccurred, this, &Connection::wsError);
	connect(qwsSocket, &QWebSocket::disconnected, this, &Connection::wsDisconnected);
	connect(qwsSocket, QOverload< const QList< QSslError > & >::of(&QWebSocket::sslErrors), this,
			&Connection::wsSslErrors);
	qtLastPacket.restart();
#ifdef Q_OS_WIN
	dwFlow = 0;
#endif
}
#endif // USE_WEBSOCKET

Connection::~Connection() {
#ifdef Q_OS_WIN
	if (dwFlow && hQoS) {
		if (!QOSRemoveSocketFromFlow(hQoS, 0, dwFlow, 0))
			qWarning("Connection: Failed to remove flow from QoS");
	}
#endif
}

void Connection::setToS() {
#if defined(Q_OS_WIN)
	if (dwFlow || !hQoS)
		return;

#	ifdef USE_WEBSOCKET
	if (qwsSocket) {
		// No raw socket descriptor available for QWebSocket; skip QoS setup.
		return;
	}
#	endif

	dwFlow = 0;
	if (!QOSAddSocketToFlow(hQoS, qtsSocket->socketDescriptor(), nullptr, QOSTrafficTypeAudioVideo,
							QOS_NON_ADAPTIVE_FLOW, reinterpret_cast< PQOS_FLOWID >(&dwFlow)))
		qWarning("Connection: Failed to add flow to QOS");
#elif defined(Q_OS_UNIX)
#	ifdef USE_WEBSOCKET
	if (qwsSocket) {
		return;
	}
#	endif
	int val = 0xa0;
	if (setsockopt(static_cast< int >(qtsSocket->socketDescriptor()), IPPROTO_IP, IP_TOS, &val, sizeof(val))) {
		val = 0x60;
		if (setsockopt(static_cast< int >(qtsSocket->socketDescriptor()), IPPROTO_IP, IP_TOS, &val, sizeof(val)))
			qWarning("Connection: Failed to set TOS for TCP Socket");
	}
#	if defined(SO_PRIORITY)
	socklen_t optlen = sizeof(val);
	if (getsockopt(static_cast< int >(qtsSocket->socketDescriptor()), SOL_SOCKET, SO_PRIORITY, &val, &optlen) == 0) {
		if (val == 0) {
			val = 6;
			setsockopt(static_cast< int >(qtsSocket->socketDescriptor()), SOL_SOCKET, SO_PRIORITY, &val, sizeof(val));
		}
	}
#	endif

#endif
}

qint64 Connection::activityTime() const {
	return qtLastPacket.elapsed();
}

void Connection::resetActivityTime() {
	qtLastPacket.restart();
}

/**
 * This function waits until a complete package is received and then emits it as a message.
 * It gets called everytime new data is available and interprets the message prefix header
 * to figure out the type and length. It then waits until the complete message is buffered
 * and emits it as a message so it can be handled by the corresponding message handler
 * routine.
 *
 * @see QSslSocket::readyRead()
 * @see void ServerHandler::message(unsigned int msgType, const QByteArray &qbaMsg)
 * @see void Server::message(unsigned int uiType, const QByteArray &qbaMsg, ServerUser *u)
 */
void Connection::socketRead() {
	while (true) {
		qint64 iAvailable = qtsSocket->bytesAvailable();
		if (iPacketLength == -1) {
			if (iAvailable < 6)
				return;

			unsigned char a_ucBuffer[6];

			qtsSocket->read(reinterpret_cast< char * >(a_ucBuffer), 6);
			m_type        = static_cast< Mumble::Protocol::TCPMessageType >(qFromBigEndian< quint16 >(&a_ucBuffer[0]));
			iPacketLength = qFromBigEndian< int >(&a_ucBuffer[2]);
			iAvailable -= 6;
		}

		if ((iPacketLength == -1) || (iAvailable < iPacketLength))
			return;

		if (iPacketLength > 0x7fffff) {
			qWarning() << "Host tried to send huge packet";
			disconnectSocket(true);
			return;
		}

		QByteArray qbaBuffer = qtsSocket->read(iPacketLength);
		iPacketLength        = -1;
		iAvailable -= iPacketLength;

		emit message(m_type, qbaBuffer);
	}
}

void Connection::socketError(QAbstractSocket::SocketError err) {
	emit connectionClosed(err, qtsSocket->errorString());
}

void Connection::socketSslErrors(const QList< QSslError > &qlErr) {
	emit handleSslErrors(qlErr);
}

void Connection::proceedAnyway() {
#ifdef USE_WEBSOCKET
	if (qwsSocket) {
		qwsSocket->ignoreSslErrors();
		return;
	}
#endif
	qtsSocket->ignoreSslErrors();
}

void Connection::socketDisconnected() {
	emit connectionClosed(QAbstractSocket::UnknownSocketError, QString());
}

#ifdef USE_WEBSOCKET
void Connection::wsBinaryMessageReceived(const QByteArray &message) {
	// Append to read buffer and parse framed Mumble packets.
	// Each WebSocket message may carry one or more protocol frames.
	m_wsReadBuffer.append(message);

	while (true) {
		if (m_wsReadBuffer.size() < 6)
			return;

		const unsigned char *hdr = reinterpret_cast< const unsigned char * >(m_wsReadBuffer.constData());
		if (iPacketLength == -1) {
			m_type        = static_cast< Mumble::Protocol::TCPMessageType >(qFromBigEndian< quint16 >(hdr));
			iPacketLength = qFromBigEndian< qint32 >(hdr + 2);
		}

		if (iPacketLength > 0x7fffff) {
			qWarning("Connection(WS): host tried to send huge packet (%d bytes)", iPacketLength);
			disconnectSocket(true);
			return;
		}

		if (m_wsReadBuffer.size() < 6 + iPacketLength)
			return;

		QByteArray payload = m_wsReadBuffer.mid(6, iPacketLength);
		m_wsReadBuffer.remove(0, 6 + iPacketLength);
		iPacketLength = -1;

		qtLastPacket.restart();
		emit message(m_type, payload);
	}
}

void Connection::wsError(QAbstractSocket::SocketError error) {
	emit connectionClosed(error, qwsSocket->errorString());
}

void Connection::wsDisconnected() {
	emit connectionClosed(QAbstractSocket::UnknownSocketError, QString());
}

void Connection::wsSslErrors(const QList< QSslError > &errors) {
	emit handleSslErrors(errors);
}
#endif // USE_WEBSOCKET

void Connection::messageToNetwork(const ::google::protobuf::Message &msg, Mumble::Protocol::TCPMessageType msgType,
								  QByteArray &cache) {
#if GOOGLE_PROTOBUF_VERSION >= 3004000
	std::size_t len = msg.ByteSizeLong();
#else
	// ByteSize() has been deprecated as of protobuf v3.4
	std::size_t len = msg.ByteSize();
#endif
	if (len > 0x7fffff)
		return;
	cache.resize(static_cast< int >(len + 6));
	unsigned char *uc = reinterpret_cast< unsigned char * >(cache.data());
	qToBigEndian< quint16 >(static_cast< quint16 >(msgType), &uc[0]);
	qToBigEndian< quint32 >(static_cast< unsigned int >(len), &uc[2]);

	msg.SerializeToArray(uc + 6, static_cast< int >(len));
}

void Connection::sendMessage(const ::google::protobuf::Message &msg, Mumble::Protocol::TCPMessageType msgType,
							 QByteArray &cache) {
	if (cache.isEmpty()) {
		messageToNetwork(msg, msgType, cache);
	}

	sendMessage(cache);
}

void Connection::sendMessage(const QByteArray &qbaMsg) {
	if (qbaMsg.isEmpty())
		return;
#ifdef USE_WEBSOCKET
	if (qwsSocket) {
		qwsSocket->sendBinaryMessage(qbaMsg);
		return;
	}
#endif
	qtsSocket->write(qbaMsg);
}

void Connection::forceFlush() {
#ifdef USE_WEBSOCKET
	if (qwsSocket) {
		qwsSocket->flush();
		return;
	}
#endif
	if (qtsSocket->state() != QAbstractSocket::ConnectedState)
		return;

	if (!qtsSocket->isEncrypted())
		return;

	qtsSocket->flush();
}

void Connection::disconnectSocket(bool force) {
#ifdef USE_WEBSOCKET
	if (qwsSocket) {
		if (force) {
			qwsSocket->abort();
		} else {
			qwsSocket->close();
		}
		return;
	}
#endif
	if (qtsSocket->state() == QAbstractSocket::UnconnectedState) {
		emit connectionClosed(QAbstractSocket::UnknownSocketError, QString());
		return;
	}

	if (force)
		qtsSocket->abort();
	else
		qtsSocket->disconnectFromHost();
}

QHostAddress Connection::peerAddress() const {
#ifdef USE_WEBSOCKET
	if (qwsSocket)
		return qwsSocket->peerAddress();
#endif
	return qtsSocket->peerAddress();
}

quint16 Connection::peerPort() const {
#ifdef USE_WEBSOCKET
	if (qwsSocket)
		return qwsSocket->peerPort();
#endif
	return qtsSocket->peerPort();
}

QHostAddress Connection::localAddress() const {
#ifdef USE_WEBSOCKET
	if (qwsSocket)
		return qwsSocket->localAddress();
#endif
	return qtsSocket->localAddress();
}

quint16 Connection::localPort() const {
#ifdef USE_WEBSOCKET
	if (qwsSocket)
		return qwsSocket->localPort();
#endif
	return qtsSocket->localPort();
}

QList< QSslCertificate > Connection::peerCertificateChain() const {
#ifdef USE_WEBSOCKET
	if (qwsSocket)
		return qwsSocket->sslConfiguration().peerCertificateChain();
#endif
	// The documentation of QSslSocket::peerCertificateChain() actually says nothing
	// about the order of the certificates in the chain. The sentence in this functions
	// documentation is taken from QSslConfiguration::peerCertificateChain().
	// Through tests and by looking into Qt's source code it was validated,
	// that these two functions do the same thing.
	// See mumble-voip/mumble#5280 for more information.
	return qtsSocket->peerCertificateChain();
}

QSslCipher Connection::sessionCipher() const {
#ifdef USE_WEBSOCKET
	if (qwsSocket)
		return qwsSocket->sslConfiguration().sessionCipher();
#endif
	return qtsSocket->sessionCipher();
}

QSsl::SslProtocol Connection::sessionProtocol() const {
#ifdef USE_WEBSOCKET
	if (qwsSocket)
		return qwsSocket->sslConfiguration().sessionProtocol();
#endif
	return qtsSocket->sessionProtocol();
}

QString Connection::sessionProtocolString() const {
	return MumbleSSL::protocolToString(sessionProtocol());
}

#ifdef Q_OS_WIN
void Connection::setQoS(HANDLE hParentQoS) {
	hQoS = hParentQoS;
}
#endif
